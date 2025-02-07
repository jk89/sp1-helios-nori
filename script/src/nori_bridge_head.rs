use alloy_primitives::{FixedBytes, B256};
use helios_consensus_core::calc_sync_period;
use helios_consensus_core::consensus_spec::MainnetConsensusSpec;
use helios_consensus_core::types::{FinalityUpdate, OptimisticUpdate};
use helios_ethereum::rpc::ConsensusRpc;
use helios_ethereum::consensus::Inner;
use helios_ethereum::rpc::http_rpc::HttpRpc;
use reqwest::Url;
use serde_cbor::ser;
use sp1_helios_primitives::types::ProofInputs;
use sp1_sdk::{EnvProver, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin};
use std::io::Write;
use std::{env, fs::File, io::Read, path::Path}; // Explicitly import the Write trait
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use crate::utils::{get_checkpoint, get_client, get_finality_updates, handle_nori_proof};
use std::fmt;
use anyhow::Result;
use tree_hash::TreeHash;
use alloy::providers::Provider;

use crate::*;


const ELF: &[u8] = include_bytes!("../../elf/sp1-helios-elf");

#[derive(PartialEq)] 
pub enum NoriBridgeHeadMode {
    Finality = 1,
    Optimistic = 2
}

impl FromStr for NoriBridgeHeadMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "finality" => Ok(NoriBridgeHeadMode::Finality),
            "optimistic" => Ok(NoriBridgeHeadMode::Optimistic),
            _ => Err(format!("Invalid value for NoriBridgeHeadMode: '{}'. Expected 'finality' or 'optimistic'.", s)),
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
#[derive(Serialize, Deserialize)] // Derive Serialize and Deserialize
pub struct NoriBridgeCheckpoint {
    slot: u64,
}

enum NoriBridgeUpdate {
    Finality(FinalityUpdate<MainnetConsensusSpec>),
    Optimistic(OptimisticUpdate<MainnetConsensusSpec>),
}

pub struct NoriBridgeHead {
    slot: u64,
    nb_checkpoint_location: String,
    rpc_url: Url,
    helios_client: Inner<MainnetConsensusSpec, HttpRpc>,
    bridge_mode: NoriBridgeHeadMode,
    next_sync_committee: FixedBytes<32>,
    pk: SP1ProvingKey,
    prover_client: EnvProver
    /*

    checkpoint: FixedBytes<32>,
    */
}

impl NoriBridgeHead {
    pub async fn new() -> Self {
        dotenv::dotenv().ok();
        let rpc_url: Url = env::var("DEST_RPC_URL")
            .expect("DEST_RPC_URL not set")
            .parse()
            .unwrap();
        let mut slot: u64 = env::var("NORI_BRIDGE_COLD_START_HEAD")
            .expect("NORI_BRIDGE_COLD_START_HEAD not set")
            .parse()
            .unwrap(); // e.g. 11008128
        let nb_checkpoint_location = env::var("NORI_BRIDGE_CHECKPOINT_FILE")
            .expect("NORI_BRIDGE_CHECKPOINT_FILE not set");
        let bridge_mode: NoriBridgeHeadMode = env::var("NORI_BRIDGE_HEAD_MODE")
            .expect("NORI_BRIDGE_HEAD_MODE not set")
            .parse()
            .unwrap();

        if bridge_mode == NoriBridgeHeadMode::Optimistic {
            panic!("Nori bridge mode Optimistic is not currently supported");
        }

        // Warm start procedure
        if NoriBridgeHead::nb_checkpoint_exists(&nb_checkpoint_location) {
            info!("Loading nori slot checkpoint from file.");
            let nb_checkpoint = NoriBridgeHead::load_nb_checkpoint(&nb_checkpoint_location);
            slot = nb_checkpoint.slot;
        } else {
            info!("Resorting to COLD_START_HEAD checkpoint env var.");
        }
        

        info!("Starting nori bridge in '{}' mode", bridge_mode);
        info!("Starting beacon client");

        // Get beacon checkpoint
        let helios_checkpoint = get_checkpoint(slot).await;

        // Get the client from the beacon checkpoint
        let helios_client = get_client(helios_checkpoint).await;

        // Get sync commitee
        let mut sync_committee_updates = get_finality_updates(&helios_client).await; // check this
        let next_sync_committee = B256::from_slice(
            sync_committee_updates[0]
                .next_sync_committee
                .tree_hash_root()
                .as_ref(),
        );

        // Get prover client
        let prover_client = ProverClient::from_env();
        // Get pk
        let (pk, _) = prover_client.setup(ELF);

        Self {
            slot,
            nb_checkpoint_location,
            rpc_url,
            helios_client,
            bridge_mode,
            next_sync_committee,
            pk,
            prover_client
        }
    }

    pub async fn get_next_finality_update(&self) -> FinalityUpdate<MainnetConsensusSpec> {
        let finality_update: FinalityUpdate<MainnetConsensusSpec> = self.helios_client.rpc.get_finality_update().await.unwrap();
        finality_update
    }

    pub async fn process_next_finality_update(&mut self) -> Result<()> {
        info!("Getting finality update");
        let finality_update = self.get_next_finality_update().await;
        let latest_slot = finality_update.finalized_header.beacon().slot;

        // If we have not evolved skip
        if (latest_slot == self.slot) {
            info!("Nori {} is up to date.", self.bridge_mode.to_string());
            return Ok(());
        }

        info!("Getting sync commitee updates");
        let mut sync_committee_updates = get_finality_updates(&self.helios_client).await;

        // Optimization:
        // Skip processing update inside program if next_sync_committee is already stored in contract.
        // We must still apply the update locally to "sync" the helios client, this is due to
        // next_sync_committee not being stored when the helios client is bootstrapped.
        info!("Applying sync committee optimisation.");
        let mut next_sync_committee: FixedBytes<32> = FixedBytes::<32>::default(); 
        let mut sync_committee_updates_not_empty = false;
        if !sync_committee_updates.is_empty() {
            sync_committee_updates_not_empty = true;
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
         handle_nori_proof(&proof, latest_slot).await?;

         // Update our state
         println!("Moving nori head forward.");
         self.slot = latest_slot;
         if sync_committee_updates_not_empty {
            self.next_sync_committee = next_sync_committee;
         }
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

        let nb_checkpoint: NoriBridgeCheckpoint = serde_cbor::from_slice(&serialized_checkpoint)
            .expect("Failed to deserialize nori checkpoint");
        nb_checkpoint
    }

    pub fn save_nb_checkpoint(&self) {
        let checkpoint = NoriBridgeCheckpoint { slot: self.slot };

        // Serialize the checkpoint to a byte vector
        let serialized_nb_checkpoint =
            serde_cbor::to_vec(&checkpoint).expect("Failed to serialize nori checkpoint");

        // Write the serialized data to the file specified by `checkpoint_location`
        let mut file = File::create(&self.nb_checkpoint_location)
            .expect("Failed to create nori checkpoint file.");
        file.write_all(&serialized_nb_checkpoint)
            .expect("Failed to write to checkpoint file.");

        info!("Nori bridge checkpoint saved successfully.");
    }
}
