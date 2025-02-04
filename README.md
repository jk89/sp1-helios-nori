# SP1 Helios

## Overview

SP1 Helios verifies the consensus of a source chain in the execution environment of a destination chain. For example, you can run an SP1 Helios light client on Polygon that verifies Ethereum Mainnet's consensus.

[Docs](https://succinctlabs.github.io/sp1-helios/)

### Skip simulation

To skip the simulation step and directly submit the program for proof generation, you can set the SKIP_SIMULATION environment variable to true. This will save some time if you are sure that your program is correct. If your program panics, the proof will fail and ProverClient will panic.

## Enabling AVX256

To enable AVX256 acceleration, you can set the RUSTFLAGS environment variable to include the following flags:

`RUSTFLAGS="-C target-cpu=native" cargo run --release`

### Performance

For maximal performance, you should run proof generation with the following command and vary your shard_size depending on your program's number of cycles.

`SHARD_SIZE=4194304 RUST_LOG=info RUSTFLAGS='-C target-cpu=native' cargo run --release`

### Memory Usage

To reduce memory usage, set the SHARD_BATCH_SIZE environment variable depending on how much RAM your machine has. A higher number will use more memory, but will be faster.

`SHARD_BATCH_SIZE=1 SHARD_SIZE=2097152 RUST_LOG=info RUSTFLAGS='-C target-cpu=native' cargo run --release`

### TODO

-use NetowkorkProver directly by using `sp1_sdk::NetworkProver`
-make client bootstrap not from sepolia contract but local/cluster proofs data with fallback to Mina state
-add logger to the operator