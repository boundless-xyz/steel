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

mod common;

use crate::common::{execute_preflight, prove_and_verify, test_config};
use alloy::providers::Provider;
use anyhow::Result;
use risc0_steel::{
    ethereum::{EthEvmEnv, EthEvmInput},
    host::BlockNumberOrTag,
};
use test_log::test;

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_block_commit() -> Result<()> {
    let cfg = test_config().await;

    let mut env = EthEvmEnv::builder()
        .provider(cfg.provider())
        .chain_spec(cfg.chain_spec)
        .block_number_or_tag(BlockNumberOrTag::Parent)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::Block(..)));
    prove_and_verify(cfg, input).await
}

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_old_block_commit() -> Result<()> {
    let cfg = test_config().await;
    let provider = cfg.provider();

    let latest = provider.get_block_number().await?;
    // Execution: 260 blocks ago, so that the blockhash opcode cannot be used to verify
    let exec_block = latest.saturating_sub(260);

    let mut env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(cfg.chain_spec)
        .block_number(exec_block)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::Block(..)));
    prove_and_verify(cfg, input).await
}

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_beacon_commit() -> Result<()> {
    let cfg = test_config().await;

    let mut env = EthEvmEnv::builder()
        .provider(cfg.provider())
        .chain_spec(cfg.chain_spec)
        .beacon_api(cfg.beacon_api.clone())
        .block_number_or_tag(BlockNumberOrTag::Parent)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::Beacon(..)));
    prove_and_verify(cfg, input).await
}

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_consensus_commit() -> Result<()> {
    let cfg = test_config().await;

    let mut env = EthEvmEnv::builder()
        .provider(cfg.provider())
        .chain_spec(cfg.chain_spec)
        .beacon_api(cfg.beacon_api.clone())
        .consensus_commitment()
        .block_number_or_tag(BlockNumberOrTag::Parent)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::Beacon(..)));
    let err = prove_and_verify(cfg, input).await.expect_err("should fail");
    assert!(
        err.to_string()
            .contains("ConsensusSlotCommitmentNotSupported"),
        "Unexpected error: {err:#}"
    );

    Ok(())
}

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_history_commit() -> Result<()> {
    let cfg = test_config().await;
    let provider = cfg.provider();

    let latest = provider.get_block_number().await?;
    let exec_block = latest.saturating_sub(10_000);

    let mut env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(cfg.chain_spec)
        .beacon_api(cfg.beacon_api.clone())
        .block_number(exec_block)
        .commitment_block_number_or_tag(BlockNumberOrTag::Parent)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::History(..)));
    prove_and_verify(cfg, input).await
}

#[test(tokio::test)]
#[cfg_attr(no_auth, ignore = "RPC tests are disabled")]
async fn test_eip_history_commit() -> Result<()> {
    let cfg = test_config().await;
    let provider = cfg.provider();

    let latest = provider.get_block_number().await?;
    let exec_block = latest.saturating_sub(10_000);

    let mut env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(cfg.chain_spec)
        .block_number(exec_block)
        .commitment_block_number_or_tag(BlockNumberOrTag::Parent)
        .build()
        .await?;
    execute_preflight(cfg, &mut env).await?;

    let input = env.into_input().await?;
    assert!(matches!(input, EthEvmInput::EipHistory(..)));
    prove_and_verify(cfg, input).await
}
