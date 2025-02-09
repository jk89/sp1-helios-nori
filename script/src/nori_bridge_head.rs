use crate::utils::{
    get_checkpoint, get_client, get_finality_updates, get_latest_checkpoint, handle_nori_proof,
};
use crate::*;
use alloy::providers::Provider;
use alloy_primitives::{FixedBytes, B256, U256};
use anyhow::Result;
use helios_consensus_core::calc_sync_period;
use helios_consensus_core::consensus_spec::MainnetConsensusSpec;
use helios_consensus_core::types::{FinalityUpdate, OptimisticUpdate};
use helios_ethereum::consensus::Inner;
use helios_ethereum::rpc::http_rpc::HttpRpc;
use helios_ethereum::rpc::ConsensusRpc;
use log::{error, info};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_cbor::ser;
use sp1_helios_primitives::types::{ProofInputs, ProofOutputs};
use sp1_sdk::{
    EnvProver, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1PublicValues, SP1Stdin,
};
use std::io::Write;
use std::str::FromStr;
use std::{default, fmt};
use std::{env, fs::File, io::Read, path::Path}; // Explicitly import the Write trait
use tree_hash::TreeHash;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustProofOutputs {
    pub execution_state_root: B256,
    pub new_header: B256,
    pub next_sync_committee_hash: B256,
    pub new_head: U256, // Use U256 instead of FixedBytes<32>
    pub prev_header: B256,
    pub prev_head: U256, // Use U256 instead of FixedBytes<32>
    pub sync_committee_hash: B256,
}

impl RustProofOutputs {
    pub fn from_abi(bytes: &[u8]) -> Result<Self> {
        // Ensure the bytes have the correct length (224 bytes for all fields)
        if bytes.len() != 224 {
            return Err(anyhow::anyhow!("Invalid byte slice length"));
        }

        // Extract the byte slices for each field
        let execution_state_root = B256::from_slice(&bytes[0..32]);
        let new_header = B256::from_slice(&bytes[32..64]);
        let next_sync_committee_hash = B256::from_slice(&bytes[64..96]);

        // Convert uint256 (big-endian bytes) into U256
        let new_head_bytes: [u8; 32] = bytes[96..128]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert new_head"))?;

        let new_head = U256::from_be_bytes(new_head_bytes);

        let prev_header = B256::from_slice(&bytes[128..160]);
        let prev_head_bytes: [u8; 32] = bytes[160..192]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert prev_head"))?;

        let prev_head = U256::from_be_bytes(prev_head_bytes);
        let sync_committee_hash = B256::from_slice(&bytes[192..224]);

        // Return the decoded struct
        Ok(RustProofOutputs {
            execution_state_root,
            new_header,
            next_sync_committee_hash,
            new_head, // Properly converted to U256
            prev_header,
            prev_head, // Properly converted to U256
            sync_committee_hash,
        })
    }
}

const SECONDS_PER_SLOT: u64 = 12;
const SLOTS_PER_EPOCH: u64 = 32;
const SLOTS_PER_PERIOD: u64 = SLOTS_PER_EPOCH * 256;
const ELF: &[u8] = include_bytes!("../../elf/sp1-helios-elf");

#[derive(PartialEq)]
pub enum NoriBridgeHeadMode {
    Finality = 1,
    Optimistic = 2,
}

impl FromStr for NoriBridgeHeadMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "finality" => Ok(NoriBridgeHeadMode::Finality),
            "optimistic" => Ok(NoriBridgeHeadMode::Optimistic),
            _ => Err(format!(
                "Invalid value for NoriBridgeHeadMode: '{}'. Expected 'finality' or 'optimistic'.",
                s
            )),
        }
    }
}

impl fmt::Display for NoriBridgeHeadMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NoriBridgeHeadMode::Finality => write!(f, "Finality"),
            NoriBridgeHeadMode::Optimistic => write!(f, "Optimistic"),
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct NoriBridgeCheckpoint {
    slot_head: u64,
    current_sync_commitee: FixedBytes<32>,
    next_sync_committee: FixedBytes<32>,
}

enum NoriBridgeUpdate {
    Finality(FinalityUpdate<MainnetConsensusSpec>),
    Optimistic(OptimisticUpdate<MainnetConsensusSpec>),
}

pub struct NoriBridgeHead {
    slot_head: u64,
    next_sync_committee: FixedBytes<32>,
    current_sync_commitee: FixedBytes<32>,
    nb_checkpoint_location: String,
    rpc_url: Url,
    helios_client: Inner<MainnetConsensusSpec, HttpRpc>,
    bridge_mode: NoriBridgeHeadMode,
    pk: SP1ProvingKey,
    prover_client: EnvProver,
}

impl NoriBridgeHead {
    pub async fn new() -> Self {
        dotenv::dotenv().ok();
        let rpc_url: Url = env::var("DEST_RPC_URL")
            .expect("DEST_RPC_URL not set")
            .parse()
            .unwrap();

        let nb_checkpoint_location =
            env::var("NORI_BRIDGE_CHECKPOINT_FILE").expect("NORI_BRIDGE_CHECKPOINT_FILE not set");
        let bridge_mode: NoriBridgeHeadMode = env::var("NORI_BRIDGE_HEAD_MODE")
            .expect("NORI_BRIDGE_HEAD_MODE not set")
            .parse()
            .unwrap();

        if bridge_mode == NoriBridgeHeadMode::Optimistic {
            panic!("Nori bridge mode Optimistic is not currently supported");
        }

        let mut slot_head = u64::default();
        let mut next_sync_committee = FixedBytes::<32>::default();
        let mut current_sync_commitee = FixedBytes::<32>::default();
        let mut cold_start = false;

        // Warm start procedure
        if NoriBridgeHead::nb_checkpoint_exists(&nb_checkpoint_location) {
            info!("Loading nori slot checkpoint from file.");
            let nb_checkpoint = NoriBridgeHead::load_nb_checkpoint(&nb_checkpoint_location);
            slot_head = nb_checkpoint.slot_head;
            next_sync_committee = nb_checkpoint.next_sync_committee;
            current_sync_commitee = nb_checkpoint.current_sync_commitee;
        } else {
            // Cold start procedure
            info!("Resorting to cold start procedure.");
            slot_head = NoriBridgeHead::get_cold_slot_head().await;
            cold_start = true;
        }

        info!("Starting nori bridge in '{}' mode", bridge_mode);
        info!("Starting beacon client");

        // Get beacon checkpoint
        let helios_checkpoint = get_checkpoint(slot_head).await;

        // Get the client from the beacon checkpoint
        let helios_client = get_client(helios_checkpoint).await;

        // Get sync commitee if we are cold starting (see genesis)
        if cold_start {
            current_sync_commitee = helios_client
                .store
                .current_sync_committee
                .clone()
                .tree_hash_root();
        } // lets start this as zero

        // Get prover client
        let prover_client = ProverClient::from_env();
        // Get pk
        let (pk, _) = prover_client.setup(ELF);

        Self {
            slot_head,
            next_sync_committee,
            current_sync_commitee,
            nb_checkpoint_location,
            rpc_url,
            helios_client,
            bridge_mode,
            pk,
            prover_client,
        }
    }

    pub async fn get_cold_slot_head() -> u64 {
        // Get latest beacon checkpoint
        let helios_checkpoint = get_latest_checkpoint().await; // see genesis

        // Get the client from the beacon checkpoint
        let helios_client = get_client(helios_checkpoint).await;

        // Get slot head from checkpoint
        helios_client.store.finalized_header.clone().beacon().slot
    }

    pub async fn get_sync_committee_period(self, slot: u64) -> u64 {
        slot / SLOTS_PER_PERIOD
    }

    pub async fn init_client(&mut self, slot: u64) {
        // Get latest beacon checkpoint
        let helios_checkpoint = get_checkpoint(slot).await;

        // Re init helios client
        self.helios_client = get_client(helios_checkpoint).await;
    }

    pub async fn run(&mut self) {
        loop {
            if let Err(e) = self.process_next_finality_update().await {
                eprintln!("Error processing nori bridge head update: {:?}", e);
            }
        }
    }

    pub async fn get_next_finality_update(&self) -> FinalityUpdate<MainnetConsensusSpec> {
        let finality_update: FinalityUpdate<MainnetConsensusSpec> =
            self.helios_client.rpc.get_finality_update().await.unwrap();
        finality_update
    }

    pub async fn process_next_finality_update(&mut self) -> Result<()> {
        // Re-init client
        self.init_client(self.slot_head).await;

        info!("Getting finality update");
        let finality_update = self.get_next_finality_update().await;
        let latest_slot = finality_update.finalized_header.beacon().slot;

        // If we have not evolved skip
        if latest_slot <= self.slot_head {
            info!(
                "Nori {} bridge is up to date.",
                self.bridge_mode.to_string()
            );
            return Ok(());
        } else {
            info!(
                "Nori {} bridge is stale. Update initiated...",
                self.bridge_mode.to_string()
            );
        }

        info!("Getting sync commitee updates");
        let mut sync_committee_updates = get_finality_updates(&self.helios_client).await;

        // Optimization:
        // Skip processing update inside program if next_sync_committee is already stored in contract.
        // We must still apply the update locally to "sync" the helios client, this is due to
        // next_sync_committee not being stored when the helios client is bootstrapped.
        info!("Applying sync committee optimisation.");
        let mut next_sync_committee: FixedBytes<32> = FixedBytes::<32>::default();
        if !sync_committee_updates.is_empty() {
            next_sync_committee = B256::from_slice(
                sync_committee_updates[0]
                    .next_sync_committee
                    .tree_hash_root()
                    .as_ref(),
            );

            if self.next_sync_committee == next_sync_committee {
                println!("Applying optimization, skipping sync committee update.");
                let temp_update = sync_committee_updates.remove(0);

                self.helios_client.verify_update(&temp_update).unwrap(); // Panics if not valid
                self.helios_client.apply_update(&temp_update);
            }
        }

        println!("Building sp1 proof inputs.");

        let mut stdin = SP1Stdin::new();

        // Create program inputs
        let expected_current_slot = self.helios_client.expected_current_slot();
        let inputs = ProofInputs {
            sync_committee_updates,
            finality_update,
            expected_current_slot,
            store: self.helios_client.store.clone(),
            genesis_root: self.helios_client.config.chain.genesis_root,
            forks: self.helios_client.config.forks.clone(),
        };
        let encoded_proof_inputs = serde_cbor::to_vec(&inputs)?;

        stdin.write_slice(&encoded_proof_inputs);

        // Generate proof.
        println!("Running sp1 proof.");
        let proof = self.prover_client.prove(&self.pk, &stdin).plonk().run()?;

        // Todo write this to the rabbit queue and other output handling
        handle_nori_proof(&proof, latest_slot).await?;

        // Update our state
        println!("Moving nori head forward.");

        // We need to extract the next sync committee out of the proof output
        let public_values: sp1_sdk::SP1PublicValues = proof.public_values;
        let public_values_bytes = public_values.as_slice(); // Raw bytes
        let proof_outputs = RustProofOutputs::from_abi(public_values_bytes).unwrap();

        self.slot_head = latest_slot;
        if proof_outputs.next_sync_committee_hash != FixedBytes::<32>::default() {
            self.next_sync_committee = proof_outputs.next_sync_committee_hash; // But wait! We need to check the logic in SP1Helios.sol as this can be Zeros
        }
        self.current_sync_commitee = proof_outputs.sync_committee_hash;
        self.save_nb_checkpoint();

        Ok(())
    }

    // Method to get updates based on the bridge_mode
    /*pub async fn get_updates(&self) {
        let next_slot = match &self.bridge_mode {
            NoriBridgeHeadMode::Finality => calc_sync_period::<MainnetConsensusSpec>(self.client.store.finalized_header.beacon().slot),
            NoriBridgeHeadMode::Optimistic => calc_sync_period::<MainnetConsensusSpec>(self.client.store.optimistic_header.beacon().slot)
        };
        info!("Next update slot {}", next_slot);
        let next_update = if let NoriBridgeHeadMode::Finality = &self.bridge_mode {
            let update: FinalityUpdate<MainnetConsensusSpec> = self.client.rpc.get_finality_update().await.unwrap();
        } else if let NoriBridgeHeadMode::Optimistic = &self.bridge_mode {
            let update: OptimisticUpdate<MainnetConsensusSpec> = self.client.rpc.get_optimistic_update().await.unwrap();
        } else {
            // Handle any other cases, maybe add a fallback or an error
            panic!("Unknown bridge mode");
        };

    }*/

    // Static method to check if the checkpoint file exists
    pub fn nb_checkpoint_exists(nb_checkpoint_location: &str) -> bool {
        Path::new(nb_checkpoint_location).exists()
    }

    // Static method to load the checkpoint from file
    pub fn load_nb_checkpoint(nb_checkpoint_location: &str) -> NoriBridgeCheckpoint {
        // Open the checkpoint file
        let mut file =
            File::open(nb_checkpoint_location).expect("Failed to open nori checkpoint file.");

        // Read the contents into a Vec<u8>
        let mut serialized_checkpoint = Vec::new();
        file.read_to_end(&mut serialized_checkpoint)
            .expect("Failed to read nori checkpoint file.");

        // Deserialize the checkpoint data using serde_json (not serde_cbor)
        let nb_checkpoint: NoriBridgeCheckpoint = serde_json::from_slice(&serialized_checkpoint)
            .expect("Failed to deserialize nori checkpoint");

        nb_checkpoint
    }

    pub fn save_nb_checkpoint(&self) {
        let checkpoint = NoriBridgeCheckpoint {
            slot_head: self.slot_head,
            current_sync_commitee: self.current_sync_commitee,
            next_sync_committee: self.next_sync_committee,
        };

        // Serialize the checkpoint to a byte vector
        let serialized_nb_checkpoint =
            serde_json::to_string(&checkpoint).expect("Failed to serialize nori checkpoint");

        // Write the serialized data to the file specified by `checkpoint_location`

        std::fs::write(&self.nb_checkpoint_location, &serialized_nb_checkpoint)
            .map_err(|e| anyhow::anyhow!("Failed to write to checkpoint file: {}", e))
            .unwrap();

        /*let mut file = File::create(&self.nb_checkpoint_location)
            .expect("Failed to create nori checkpoint file.");
        file.write_all(&serialized_nb_checkpoint)
            .expect("Failed to write to checkpoint file.");*/

        info!("Nori bridge checkpoint saved successfully.");
    }
}
