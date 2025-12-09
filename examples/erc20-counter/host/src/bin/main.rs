// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use erc20_counter_core::{IERC20, Input, Journal};
use erc20_counter_methods::{ERC20_COUNTER_GUEST_ELF, ERC20_COUNTER_GUEST_ID};
use risc0_ethereum_contracts::encode_seal;
use risc0_steel::{
    Contract,
    alloy::{
        network::EthereumWallet,
        providers::{Provider, ProviderBuilder},
        signers::local::PrivateKeySigner,
        sol,
        sol_types::{SolCall, SolValue},
    },
    ethereum::{EthChainSpec, EthEvmEnv},
    host::BlockNumberOrTag,
};
use risc0_zkvm::{Digest, ExecutorEnv, Prover, ProverOpts, default_prover};
use tracing_subscriber::EnvFilter;
use url::Url;

sol!(
    #[sol(rpc)]
    "../contracts/src/ICounter.sol"
);

/// Simple program to create a proof to increment the Counter contract.
#[derive(Parser)]
struct Args {
    /// Ethereum private key
    #[arg(long, env = "ETH_WALLET_PRIVATE_KEY")]
    eth_wallet_private_key: PrivateKeySigner,

    /// Ethereum RPC endpoint URL
    #[arg(long, env = "ETH_RPC_URL")]
    eth_rpc_url: Url,

    /// Ethereum block to use as the state for the contract call
    #[arg(long, env = "EXECUTION_BLOCK", default_value_t = BlockNumberOrTag::Latest)]
    execution_block: BlockNumberOrTag,

    /// Address of the Counter verifier contract
    #[arg(long)]
    counter_address: Address,

    /// Address of the ERC20 token contract
    #[arg(long)]
    token_contract: Address,

    /// Address to query the token balance of
    #[arg(long, env = "TOKEN_OWNER")]
    token_owner: Address,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if it exists
    dotenvy::dotenv().ok();
    // Initialize tracing. In order to view logs, run `RUST_LOG=info cargo run`
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    // Parse the command line arguments.
    let args = Args::try_parse()?;

    // Create an alloy provider for that private key and URL.
    let wallet = EthereumWallet::from(args.eth_wallet_private_key);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(args.eth_rpc_url);

    // Load the specification corresponding to the chain ID.
    let chain_id = provider.get_chain_id().await?;
    let chain_spec = EthChainSpec::from_chain_id(chain_id)
        .with_context(|| format!("Unsupported chain ID: {chain_id}"))?;

    // Build the corresponding environment.
    let builder = EthEvmEnv::builder()
        .provider(provider.clone())
        .chain_spec(chain_spec)
        .block_number_or_tag(args.execution_block);
    let mut env = builder.build().await?;

    // Prepare the function call
    let call = IERC20::balanceOfCall {
        account: args.token_owner,
    };

    // Preflight the call to prepare the input that is required to execute the function in
    // the guest without RPC access. It also returns the result of the call.
    let mut contract = Contract::preflight(args.token_contract, &mut env);
    let returns = contract.call_builder(&call).call().await?;
    assert!(returns >= U256::from(1));

    // Finally, construct the input from the environment.
    let evm_input = env.into_input().await?;

    let input = Input {
        chain_id,
        evm_input,
        erc20_contract: args.token_contract,
        account: args.token_owner,
    };

    // Create the RiscZERO proof.
    let prove_info = tokio::task::spawn_blocking(move || {
        let env = ExecutorEnv::builder().write(&input)?.build().unwrap();

        default_prover().prove_with_opts(env, ERC20_COUNTER_GUEST_ELF, &ProverOpts::groth16())
    })
    .await?
    .context("failed to create proof")?;
    let receipt = prove_info.receipt;
    let journal = &receipt.journal.bytes;

    // Decode and log the commitment
    let journal = Journal::abi_decode(journal).context("invalid journal")?;
    log::debug!("Steel commitment: {:?}", journal.commitment);

    // ABI encode the seal.
    let seal = encode_seal(&receipt).context("invalid receipt")?;

    // Create an alloy instance of the Counter contract.
    let contract = ICounter::new(args.counter_address, &provider);

    // Call ICounter::imageId() to check that the contract has been deployed correctly.
    let contract_image_id = contract.imageId().call().await?;
    ensure!(
        contract_image_id.0 == <[u8; 32]>::from(Digest::from(ERC20_COUNTER_GUEST_ID)),
        "image ID mismatch; redeploying the Counter contract should fix this"
    );

    // Call the increment function of the contract and wait for confirmation.
    log::info!(
        "Sending Tx calling {} Function of {:#}...",
        ICounter::incrementCall::SIGNATURE,
        contract.address()
    );
    let call_builder = contract.increment(receipt.journal.bytes.into(), seal.into());
    log::debug!("Send {} {}", contract.address(), call_builder.calldata());
    let pending_tx = call_builder.send().await?;
    let tx_hash = *pending_tx.tx_hash();
    let receipt = pending_tx
        .get_receipt()
        .await
        .with_context(|| format!("transaction did not confirm: {tx_hash}"))?;
    ensure!(receipt.status(), "transaction failed: {tx_hash}");

    let count = contract.count().call().await?;
    println!("✅ On-chain verification passed; new count: {count}");

    Ok(())
}
