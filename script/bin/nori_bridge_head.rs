use std::env;
use sp1_helios_script::{nori_bridge_head::NoriBridgeHead, utils::enable_logging_from_cargo_run};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    enable_logging_from_cargo_run();
    let mut nbh = NoriBridgeHead::new().await;
    Ok(())
}