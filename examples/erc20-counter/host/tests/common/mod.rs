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

// This application demonstrates how to send an off-chain proof request
// to the Bonsai proving service and publish the received proofs directly
// to your deployed app contract.

//! Integration test helpers for Steel ERC20 counter example.
//!
//! These tests run against live Ethereum networks (mainnet, testnets) using:
//! - State overrides to simulate contract deployment without transactions
//! - Real ERC20 tokens and holder accounts for verification
//! - Multiple commitment types (Block, Beacon, History, EIP History)
//!
//! # Environment Variables
//!
//! - `ETH_RPC_URL`: Ethereum RPC endpoint (required)
//! - `BEACON_API_URL`: Beacon chain API endpoint (required for beacon tests)

use alloy::network::Ethereum;
use alloy::{
    node_bindings::Anvil,
    providers::{Provider, ProviderBuilder},
    rpc::types::state::{AccountOverride, StateOverride, StateOverridesBuilder},
    sol,
    sol_types::SolType,
};
use alloy_primitives::{Address, B256, ChainId, U256, address};
use anyhow::{Context, Result, anyhow, bail, ensure};
use erc20_counter_core::{IERC20, Input, Journal};
use erc20_counter_methods::{ERC20_COUNTER_GUEST_ELF, ERC20_COUNTER_GUEST_ID};
use risc0_steel::ethereum::ETH_HOODI_CHAIN_SPEC;
use risc0_steel::{
    Contract,
    ethereum::{
        ETH_MAINNET_CHAIN_SPEC, ETH_SEPOLIA_CHAIN_SPEC, EthChainSpec, EthEvmEnv, EthEvmInput,
        STEEL_TEST_PRAGUE_CHAIN_SPEC,
    },
    host::{
        HostCommit,
        db::{ProofDb, ProviderDb},
    },
};
use risc0_zkvm::{Digest, ExecutorEnv, ProveInfo, ProverOpts, default_prover};
use std::{collections::BTreeMap, fmt::Debug, sync::LazyLock};
use tokio::sync::OnceCell;
use url::Url;

sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    MockVerifier,
    "../contracts/out/MockVerifier.sol/MockVerifier.json"
);
sol!(
    #[sol(rpc)]
    #[derive(Debug)]
    Counter,
    "../contracts/out/Counter.sol/Counter.json"
);

static CHAIN_TEST_DATA: LazyLock<BTreeMap<ChainId, (Address, Address)>> = LazyLock::new(|| {
    BTreeMap::from([
        (
            ETH_MAINNET_CHAIN_SPEC.chain_id,
            (
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
                address!("0xF977814e90dA44bFA03b6295A0616a897441aceC"),
            ),
        ),
        (
            ETH_SEPOLIA_CHAIN_SPEC.chain_id,
            (
                address!("0xaA8E23Fb1079EA71e0a56F48a2aA51851D8433D0"), // Sepolia USDT
                address!("0xc94b1BEe63A3e101FE5F71C80F912b4F4b055925"),
            ),
        ),
        (
            ETH_HOODI_CHAIN_SPEC.chain_id,
            (
                address!("0x499b095Ed02f76E56444c242EC43A05F9c2A3ac8"), // Hoodi Drosera (DRO)
                address!("0x780521b58Ff8fFB7df09195E79810580279a4d9d"),
            ),
        ),
        (
            STEEL_TEST_PRAGUE_CHAIN_SPEC.chain_id,
            (
                address!("0xb158cf0fd130353535210380fc136a8d31714682"), // ERC20 Toyken
                address!("0x802dCbE1B1A97554B4F50DB5119E37E8e7336417"),
            ),
        ),
    ])
});

pub async fn test_config() -> &'static TestConfig {
    static CFG: OnceCell<TestConfig> = OnceCell::const_new();

    CFG.get_or_init(TestConfig::new).await
}

pub struct TestConfig {
    eth_rpc: Url,
    pub beacon_api: Url,
    pub chain_spec: &'static EthChainSpec,
    pub counter_address: Address,
    pub erc20_address: Address,
    pub test_account: Address,
    pub contract_overrides: StateOverride,
}

impl TestConfig {
    async fn new() -> Self {
        Self::try_new().await.unwrap()
    }

    async fn try_new() -> Result<Self> {
        // Load .env file if it exists
        dotenvy::dotenv().ok();

        let eth_rpc = parse_env_url("ETH_RPC_URL")?;
        println!("ETH_RPC_URL: {eth_rpc}");
        let beacon_api = parse_env_url("BEACON_API_URL")?;
        println!("BEACON_API_URL: {beacon_api}");

        let provider = ProviderBuilder::new().connect_http(eth_rpc.clone());
        let chain_id = provider.get_chain_id().await?;
        let chain_spec = EthChainSpec::from_chain_id(chain_id)
            .with_context(|| format!("unsupported chain id: {chain_id}"))?;

        let (erc20_address, test_account) = *CHAIN_TEST_DATA.get(&chain_id).with_context(|| {
            format!(
                "Unsupported chain ID: {chain_id}. Supported: {:?}",
                CHAIN_TEST_DATA.keys().collect::<Vec<_>>()
            )
        })?;
        println!("ERC20_CONTRACT: {erc20_address}");

        let counter_address = Address::random();
        let contract_overrides = build_contract_overrides(counter_address, erc20_address)
            .await
            .context("failed to create overrides")?;

        Ok(Self {
            eth_rpc,
            beacon_api,
            chain_spec,
            counter_address,
            erc20_address,
            test_account,
            contract_overrides,
        })
    }

    pub fn provider(&self) -> impl Provider {
        ProviderBuilder::new().connect_http(self.eth_rpc.clone())
    }
}

/// Generic preflight logic. Works for any Commitment type C.
pub async fn execute_preflight<P: Provider + 'static, C>(
    cfg: &TestConfig,
    env: &mut EthEvmEnv<ProofDb<ProviderDb<Ethereum, P>>, HostCommit<C>>,
) -> Result<()> {
    let mut contract = Contract::preflight(cfg.erc20_address, env);

    let call = IERC20::balanceOfCall {
        account: cfg.test_account,
    };
    let returns = contract.call_builder(&call).call().await?;
    ensure!(
        returns >= U256::from(1),
        "Account {} has insufficient balance (needs >= 1)",
        call.account
    );

    Ok(())
}

/// Core logic: Runs the ZK Proof generation and verifies it using eth_call
pub async fn prove_and_verify(cfg: &TestConfig, evm_input: EthEvmInput) -> Result<()> {
    let input = Input {
        chain_id: cfg.chain_spec.chain_id,
        evm_input,
        erc20_contract: cfg.erc20_address,
        account: cfg.test_account,
    };

    let ProveInfo { receipt, .. } = tokio::task::spawn_blocking(move || {
        let env = ExecutorEnv::builder().write(&input)?.build().unwrap();

        let opts = ProverOpts::groth16().with_dev_mode(true);
        default_prover().prove_with_opts(env, ERC20_COUNTER_GUEST_ELF, &opts)
    })
    .await?
    .context("failed to create proof")?;

    let seal = risc0_ethereum_contracts::encode_seal(&receipt).context("invalid receipt")?;
    let journal_bytes = receipt.journal.bytes;
    let journal = Journal::abi_decode(&journal_bytes).context("invalid journal")?;

    println!("✅ Proof generated. Commitment: {:?}", journal.commitment);

    let contract = Counter::new(cfg.counter_address, cfg.provider());

    // Execute the transaction against the state overrides
    match contract
        .increment(journal_bytes.into(), seal.into())
        .call()
        .overrides(cfg.contract_overrides.clone())
        .await
    {
        Ok(_) => println!("✅ On-chain verification simulation passed"),
        Err(e) => {
            // Decode revert if possible
            if let Some(sol_error) = e.as_decoded_interface_error::<Counter::CounterErrors>() {
                bail!("call reverted: {sol_error:?}");
            }
            if let Some(sol_error) =
                e.as_decoded_interface_error::<MockVerifier::MockVerifierErrors>()
            {
                bail!("call reverted: {sol_error:?}");
            }
            return Err(anyhow!(e).context("call failed"));
        }
    }

    Ok(())
}

fn to_b256(digest: Digest) -> B256 {
    <[u8; 32]>::from(digest).into()
}

fn parse_env_url(key: &str) -> Result<Url> {
    let url_str = std::env::var(key).with_context(|| format!("{key} must be set"))?;
    Url::parse(&url_str).with_context(|| format!("{key} is not a valid url"))
}

/// Deploys contracts to a local Anvil instance and captures their bytecode for use in state overrides against live chains.
async fn build_contract_overrides(
    counter_addr: Address,
    token_addr: Address,
) -> Result<StateOverride> {
    let provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(Anvil::paris)
        .context("failed to start Anvil")?;

    let verifier_addr = Address::random();
    let verifier = MockVerifier::deploy(&provider)
        .await
        .context("verifier deploy failed")?;
    let verifier_code = provider.get_code_at(*verifier.address()).await?;

    let image_id = to_b256(ERC20_COUNTER_GUEST_ID.into());
    let counter = Counter::deploy(&provider, verifier_addr, token_addr, image_id)
        .await
        .context("counter deploy failed")?;
    let counter_code = provider.get_code_at(*counter.address()).await?;

    let overrides = StateOverridesBuilder::default()
        .append(
            verifier_addr,
            AccountOverride::default().with_code(verifier_code),
        )
        .append(
            counter_addr,
            AccountOverride::default().with_code(counter_code),
        )
        .build();

    Ok(overrides)
}
