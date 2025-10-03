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
2025-10-03T16:42:49.978575Z DEBUG risc0_steel::host::builder: Environment initialized with block 23494919 (0xc77d23de17ba1e8c553a40f85389e51af24e1cd859226f72fd7c45b24ad3a311)
2025-10-03T16:42:49.978795Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getUtilization()'
Call getUtilization() Function on 0xc3d6…cdc3 returns: 909134506353272671
2025-10-03T16:42:50.692854Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getSupplyRate(uint256)'
Call getSupplyRate(uint256) Function on 0xc3d6…cdc3 returns: 1953129194
2025-10-03T16:42:51.386295Z DEBUG risc0_steel::host::builder: Environment initialized with block 23498519 (0x5ae45981786f213026c2d8c378e7bf1779b090f1f6e4a324bf15208bd148d969)
2025-10-03T16:42:51.386344Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getUtilization()'
Call getUtilization() Function on 0xc3d6…cdc3 returns: 904841498272350121
2025-10-03T16:42:52.113304Z DEBUG risc0_steel::contract::host: Executing preflight calling 'getSupplyRate(uint256)'
Call getSupplyRate(uint256) Function on 0xc3d6…cdc3 returns: 1518056457
2025-10-03T16:42:52.697573Z DEBUG risc0_steel::verifier::host: Executing preflight verifying: Commitment { version: "Block", id: 23494919, digest: 0xc77d23de17ba1e8c553a40f85389e51af24e1cd859226f72fd7c45b24ad3a311, configID: 0x9a223c7ca04c969f1cacbe5b8db44c308b2c53390505d3d48c834ed4469fc839 }
2025-10-03T16:42:52.697612Z DEBUG risc0_steel::contract::host: Executing preflight calling 'raw'
2025-10-03T16:42:54.061190Z DEBUG risc0_steel::block::host: Preparing input for block 23494919 (0xc77d23de17ba1e8c553a40f85389e51af24e1cd859226f72fd7c45b24ad3a311):
2025-10-03T16:42:55.406215Z DEBUG risc0_steel::block::host: Preparing input for block 23498519 (0x5ae45981786f213026c2d8c378e7bf1779b090f1f6e4a324bf15208bd148d969):
Running the guest with the constructed input:
2025-10-03T16:42:55.895463Z  INFO risc0_zkvm::host::server::exec::executor: execution time: 79.894667ms
Commitment { version: "Block", id: 23498519, digest: 0x5ae45981786f213026c2d8c378e7bf1779b090f1f6e4a324bf15208bd148d969, configID: 0x9a223c7ca04c969f1cacbe5b8db44c308b2c53390505d3d48c834ed4469fc839 }
Proven APR calculated is: 5.4733655344968%
```

[install-rust]: https://doc.rust-lang.org/cargo/getting-started/installation.html
[install-risczero]: https://dev.risczero.com/api/zkvm/install
