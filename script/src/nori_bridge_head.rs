use alloy_primitives::FixedBytes;
use helios_consensus_core::consensus_spec::MainnetConsensusSpec;
use helios_ethereum::consensus::Inner;
use helios_ethereum::rpc::http_rpc::HttpRpc;
use reqwest::Url;
use serde_cbor::ser;
use sp1_sdk::{EnvProver, ProverClient, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin};
use std::io::Write;
use std::{env, fs::File, io::Read, path::Path}; // Explicitly import the Write trait

use log::{error, info};
use serde::{Deserialize, Serialize};

use crate::utils::{get_checkpoint, get_client};

#[derive(Serialize, Deserialize)] // Derive Serialize and Deserialize
pub struct NoriBridgeCheckpoint {
    slot: u64,
}

pub struct NoriBridgeHead {
    slot: u64,
    nb_checkpoint_location: String,
    rpc_url: Url,
    client: Inner<MainnetConsensusSpec, HttpRpc>
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
        let mut slot: u64 = env::var("COLD_START_HEAD")
            .expect("COLD_START_HEAD not set")
            .parse()
            .unwrap(); // e.g. 11008128
        let nb_checkpoint_location =
            env::var("NORI_BRIGE_CHECKPOINT_FILE").expect("NORI_BRIGE_CHECKPOINT_FILE not set");

        // Warm start procedure
        if NoriBridgeHead::nb_checkpoint_exists(&nb_checkpoint_location) {
            info!("Loading nori slot checkpoint from file.");
            let nb_checkpoint = NoriBridgeHead::load_nb_checkpoint(&nb_checkpoint_location);
            slot = nb_checkpoint.slot;
        } else {
            info!("Resorting to COLD_START_HEAD checkpoint env var.");
        }

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
        }
    }

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
