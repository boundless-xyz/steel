# ERC20-Counter Example

This example implements a counter that increments based on off-chain RISC Zero [Steel] proofs submitted to the [Counter] contract.
The contract interacts with ERC-20 tokens, using [Steel] proofs to verify that an account holds at least 1 token before incrementing the counter.

## Overview

The [Counter] contract is designed to interact with the Ethereum blockchain, leveraging the power of RISC Zero [Steel] proofs to perform a specific operation: incrementing a counter based on the token holdings of an account.

### Contract Functionality

#### Increment Counter

The core functionality of the [Counter] contract is to increment an internal counter whenever a valid proof was submitted.
This proof must demonstrate that a specified account holds at least one unit of a particular ERC-20 token.
The contract ensures that the counter is only incremented when the proof is verified and the condition of holding at least one token is met.

#### Steel Proof Submission

Users or entities can submit proofs to the [Counter] contract.
These proofs are generated off-chain using the RISC Zero zkVM.
The proof encapsulates the verification of an account's token balance without exposing the account's details or requiring direct on-chain queries.

#### Token Balance Verification

Upon receiving a [Steel] proof, the [Counter] contract decodes the proof and validates it against the contract's state at a certain block height.
This ensures that the account in question actually holds at least one token at the time the proof was generated.

#### Counter Management

The contract maintains an internal counter, which is publicly viewable.
This counter represents the number of successful verifications that have occurred.
The contract includes functionality to query the current value of the counter at any time.

## Dependencies

To get started, you need to have the following installed:

- [Rust]
- [Foundry]
- [RISC Zero]

## Deploy Your Application

You can either:

- [Deploy on Anvil]
- [Deploy on a testnet]

### Deploy on Anvil

You can deploy your contracts and run an end-to-end test or demo as follows:

1. Start a local testnet with `anvil` by running:

    ```bash
    anvil --chain-id 5733100018 --hardfork prague
    ```

   This starts anvil with the preconfigured Steel test network ID.
   Once anvil is started, keep it running in the terminal, and switch to a new terminal.

2. Set your environment variables:

    ```bash
    # Anvil sets up a number of default wallets, and this private key is one of them.
    export ETH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
    ```
3. Build the Project:
    ```bash
    cargo build
    ```

4. Deploy the Counter contract. During creation, the Counter gets linked with an ERC20 token. To also deploy such a new token, you need to specify any `TOKEN_OWNER` address which will get funded with Toyken ERC20 tokens, for example the address of the private key:
    ```bash
    export TOKEN_OWNER=
    ```
   Then, deploy the contracts running the following script:
    ```bash
    RISC0_DEV_MODE=true forge script --rpc-url http://localhost:8545 --broadcast DeployCounter
    ```
   This command should output something similar to:

    ```bash
    == Logs ==
      Deployed ERC20 TOYKEN to 0x5FbDB2315678afecb367f032d93F642f64180aa3
      Account 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 has balance: 1000
      Deployed RiscZeroMockVerifier to 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
      Deployed Counter to 0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
    ...
    ```
   Save the `ERC20 Toyken` contract address to an env variable:
    ```bash
    export TOYKEN_ADDRESS=#COPY ERC20 TOYKEN ADDRESS FROM DEPLOY LOGS
    ```

   Save the `Counter` contract address to an env variable:

    ```bash
    export COUNTER_ADDRESS=#COPY COUNTER ADDRESS FROM DEPLOY LOGS
    ```

#### Interact with your local deployment

1. Query the state:

    ```bash
    cast call --rpc-url http://localhost:8545 $COUNTER_ADDRESS 'count()(uint256)'
    ```

2. Publish a new state

    ```bash
    RUST_LOG=info RISC0_DEV_MODE=true cargo run -- \
        --eth-rpc-url=http://localhost:8545 \
        --counter-address=$COUNTER_ADDRESS \
        --token-contract=$TOYKEN_ADDRESS
    ```

3. Query the state again to see the change:

    ```bash
    cast call --rpc-url http://localhost:8545 $COUNTER_ADDRESS 'count()(uint256)'
    ```

### Deploy your project on a public network

You can deploy the Counter contract on any Ethereum network such as `Sepolia` and run an end-to-end test or demo as follows:
> ***Note***: we'll be using an existing ERC20 contract for this example, specifically the USDT ERC20 contract deployed on Sepolia at address [0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0].

1. Get access to Bonsai and an Ethereum node running on a given testnet, e.g., Sepolia and export the following environment variables:
    ```bash
    export ETH_WALLET_PRIVATE_KEY="YOUR_WALLET_PRIVATE_KEY" # the private hex-encoded key of your Sepolia testnet wallet
    export TOKEN_CONTRACT=0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0 # Sepolia USDT
    export ETH_RPC_URL=https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY
    ```

2. Build the project:
    ```bash
    cargo build
    ```

3. Deploy the Counter contract by running:

    ```bash
    forge script --rpc-url $ETH_RPC_URL --broadcast DeployCounter
    ```

   This command should output something similar to:

    ```bash
    ...
    == Logs ==
    Using ERC20 USDT at 0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0
    Deployed RiscZeroGroth16Verifier to 0x5a1677454B5530a15536EF662C6b27b14F699aBd
    Deployed Counter to 0xb0827e4F251d29685170837C2C0eE204Dfef522c
    ...
    ```

   Save the `Counter` contract address to an env variable:

    ```bash
    export COUNTER_ADDRESS=#COPY COUNTER ADDRESS FROM DEPLOY LOGS
    ```

#### Interact with your testnet deployment

1. Query the state. It should return `0` for a newly deployed Counter contract:

    ```bash
    cast call --rpc-url $ETH_RPC_URL $COUNTER_ADDRESS 'count()(uint256)'
    ```

2. Publish a new state

    ```bash
    RUST_LOG=info cargo run -- \
        --counter-address=$COUNTER_ADDRESS \
        --token-contract=$TOKEN_CONTRACT \
        --token-owner=0x9737100D2F42a196DE56ED0d1f6fF598a250E7E4
    ```

3. Query the state again to see the change:

    ```bash
    cast call --rpc-url $ETH_RPC_URL $COUNTER_ADDRESS 'count()(uint256)'
    ```
   

[Foundry]: https://getfoundry.sh/
[Groth16 SNARK proof]: https://www.risczero.com/news/on-chain-verification
[RISC Zero]: https://dev.risczero.com/api/zkvm/install
[Sepolia]: https://www.alchemy.com/overviews/sepolia-testnet
[deployment guide]: ./deployment-guide.md
[Rust]: https://doc.rust-lang.org/cargo/getting-started/installation.html
[Counter]: ./contracts/src/Counter.sol
[Steel]: https://www.risczero.com/blog/introducing-steel
[Deploy on a testnet]: #deploy-your-project-on-a-public-network
[Deploy on a local network]: #deploy-on-anvil
[0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0]: https://sepolia.etherscan.io/address/0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0#code
