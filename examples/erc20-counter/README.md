# RISC Zero Steel: ERC-20 Counter Example

This example demonstrates how to use [Steel] to prove the result of a view call to an ERC-20 contract on Ethereum.
Specifically, the guest program checks if a specific account holds a token balance > 1. If valid, it generates a [Groth16 SNARK proof] that is submitted to the `Counter` contract on-chain, which verifies the proof and increments a counter.

## Prerequisites

Ensure you have the following installed:
- [Rust]
- [Foundry]
- [RISC Zero]
- [Docker]

## Quick Start (Local Development)

The easiest way to run this example is on a local [Anvil] chain. The deployment script will automatically set up a Mock Token and Mock Verifier for you.

### 1. Start Anvil
Open a terminal and run the following to start anvil in the correct configuration:
```bash
anvil --chain-id 5733100018 --hardfork prague
```

*Keep this terminal running.*

### 2. Deploy Contracts

Open a **new terminal**.

Set the default Anvil private key (Account #0):
```bash
export ETH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

Run the deployment script:
```bash
forge script --broadcast contracts/script/DeployCounter.s.sol \
    --rpc-url http://localhost:8545 \
    --private-key $ETH_WALLET_PRIVATE_KEY
```

You will see output similar to this.

```text
Deployed ERC20 TOKEN to 0x5FbDB2315678afecb367f032d93F642f64180aa3
Account 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 has balance: 1000
Deployed RiscZeroMockVerifier to 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
Deployed Counter to 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
...
```

### 3. Generate Proof & Submit

Run the host application using the addresses from the previous step. 

```bash
# Replace the addresses below with your deployment output
RUST_LOG=info RISC0_DEV_MODE=true cargo run -- \
    --eth-wallet-private-key $ETH_WALLET_PRIVATE_KEY \
    --eth-rpc-url http://localhost:8545 \
    --token-owner 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
    --counter-address <PASTE_COUNTER_ADDRESS> \
    --token-contract <PASTE_TOKEN_ADDRESS>
```

If successful, you will see:
`✅ On-chain verification passed ...`

-----

## Integration Tests (Stateless)

This repository includes robust integration tests that verify Steel proofs against **live networks** without requiring a wallet or local deployment. These tests use `eth_call` with state overrides to simulate the verification.

To run the test suite (including Block, History, and Beacon commitment tests):

```bash
# Requires a valid RPC URL
export ETH_RPC_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY/
# Note: The BEACON_API_URL must end with a trailing slash '/'
# Otherwise, the Rust URL parser may discard the last path segment.
export BEACON_API_URL=https://eth-mainnetbeacon.g.alchemy.com/v2/YOUR_API_KEY/

cargo test --workspace --test stateless
```

[Docker]: https://www.docker.com/get-started/
[Foundry]: https://getfoundry.sh/
[Groth16 SNARK proof]: https://www.risczero.com/news/on-chain-verification
[RISC Zero]: https://dev.risczero.com/api/zkvm/install
[Rust]: https://doc.rust-lang.org/cargo/getting-started/installation.html
[Steel]: https://www.risczero.com/blog/introducing-steel
