

pub struct NoriBridgeHead {
    rpc_url: Url,
    slot: u64,
    checkpoint: FixedBytes<32>,
    client: EnvProver
}

pub impl NoriBridgeHead {
    pub async fn new () -> Self {
        dotenv::dotenv().ok();
        let rpc_url = env::var("DEST_RPC_URL")
            .expect("DEST_RPC_URL not set")
            .parse()
            .unwrap();
        let slot: u64 = env::var("COLD_HEAD_SLOT")
            .expect("COLD_HEAD_SLOT not set")
            .parse()
            .unwrap(); // e.g. 11008128

        Self {
            
        }
    }
}