# Compound Token Stats (APR Proof)

**An example that calls multiple view functions on the Compound USDC contract to compute the APR.**

## Prerequisites

To get started, you need to have Rust installed. If you haven't done so, follow the instructions [here][install-rust].

Next, you will also need to have the `cargo-risczero` tool installed following the instructions [here][install-risczero].

You'll also need access to an Ethereum Mainnet RPC endpoint. You can for example use [ethereum-rpc.publicnode.com](https://ethereum-rpc.publicnode.com/) or a commercial RPC provider like [Alchemy](https://www.alchemy.com/).

## Run the example

To run the example, which computes the current APR of the Compound USDC Token [`0xc3d688B66703497DAA19211EEdff47f25384cdc3`](https://etherscan.io/token/0xc3d688B66703497DAA19211EEdff47f25384cdc3) on Ethereum, execute the following command:

```bash
RPC_URL=https://ethereum-rpc.publicnode.com RUST_LOG=info,risc0_steel=debug cargo run --release
```

The output should resemble the following:

```text
2025-10-07T11:35:38.730992Z DEBUG risc0_steel::host::builder: Environment initialized with block 23518416 (0xf3b179a5030338d6d3b6477843ac14027c7e6f25ab4c8d42e61c2fbec616d598)
2025-10-07T11:35:38.731380Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getUtilization()'
Call getUtilization() Function on 0xc3d6…cdc3 returns: 901643987632970446
2025-10-07T11:35:39.566357Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getSupplyRate(uint256)'
Call getSupplyRate(uint256) Function on 0xc3d6…cdc3 returns: 1194006355
2025-10-07T11:35:40.350225Z DEBUG risc0_steel::host::builder: Environment initialized with block 23525616 (0x3728e8f5dcf4d42b68d5cd424c9c7fcb0582d552f14129f94411aab8e7527746)
2025-10-07T11:35:40.350296Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getUtilization()'
Call getUtilization() Function on 0xc3d6…cdc3 returns: 809451025252486797
2025-10-07T11:35:41.266703Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getSupplyRate(uint256)'
Call getSupplyRate(uint256) Function on 0xc3d6…cdc3 returns: 924030850
2025-10-07T11:35:41.937053Z DEBUG risc0_steel::verifier::host: Executing preflight verifying Commitment { version: "Block", id: 23518416, digest: 0xf3b179a5030338d6d3b6477843ac14027c7e6f25ab4c8d42e61c2fbec616d598, configID: 0x9a223c7ca04c969f1cacbe5b8db44c308b2c53390505d3d48c834ed4469fc839 }
2025-10-07T11:35:41.937113Z DEBUG risc0_steel::contract::host: Executing preflight calling 'raw'
2025-10-07T11:35:43.367732Z DEBUG risc0_steel::block::host: Preparing input for block 23518416:
2025-10-07T11:35:44.295123Z DEBUG risc0_steel::block::host: Preparing input for block 23525616:
Running the guest with the constructed input:
2025-10-07T11:35:44.777284Z  INFO risc0_zkvm::host::server::exec::executor: execution time: 79.718292ms
Commitment { version: "Block", id: 23525616, digest: 0x3728e8f5dcf4d42b68d5cd424c9c7fcb0582d552f14129f94411aab8e7527746, configID: 0x9a223c7ca04c969f1cacbe5b8db44c308b2c53390505d3d48c834ed4469fc839 }
Proven APR over 2 days is: 3.339721064844
```

[install-rust]: https://doc.rust-lang.org/cargo/getting-started/installation.html
[install-risczero]: https://dev.risczero.com/api/zkvm/install
