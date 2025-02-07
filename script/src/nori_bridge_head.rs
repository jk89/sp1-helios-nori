use alloy_primitives::FixedBytes;
use helios_consensus_core::calc_sync_period;
use helios_consensus_core::consensus_spec::MainnetConsensusSpec;
use helios_consensus_core::types::{FinalityUpdate, OptimisticUpdate};
use helios_ethereum::rpc::ConsensusRpc;
use helios_ethereum::consensus::Inner;
use helios_ethereum::rpc::http_rpc::HttpRpc;
use reqwest::Url;
use serde_cbor::ser;
use sp1_sdk::{EnvProver, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin};
use std::io::Write;
use std::{env, fs::File, io::Read, path::Path}; // Explicitly import the Write trait
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use crate::utils::{get_checkpoint, get_client};
use std::fmt;

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
    client: Inner<MainnetConsensusSpec, HttpRpc>,
    bridge_mode: NoriBridgeHeadMode
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
        let beacon_checkpoint = get_checkpoint(slot).await;

        // Get the client from the beacon checkpoint
        let client = get_client(beacon_checkpoint).await;

        //  //nbh.save_checkpoint();
        Self {
            slot,
            nb_checkpoint_location,
            rpc_url,
            client,
            bridge_mode
        }
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

    } */

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
