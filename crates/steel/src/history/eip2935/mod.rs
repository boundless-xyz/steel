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

use crate::{
    history::state::SingleContractState, BlockHeaderCommit, Commitment, CommitmentVersion,
    ComposeInput, EvmBlockHeader, EvmFactory,
};
use alloy_primitives::{Sealed, B256};
use execution_hash::ExecutionHashContract;
use serde::{Deserialize, Serialize};

mod execution_hash;

/// Input recursively committing to multiple execution block hashes.
pub type HistoryInput<F> = ComposeInput<F, HistoryCommit<<F as EvmFactory>::Header>>;

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryCommit<H> {
    /// Iterative commits for verifying the execution block as an ancestor of some other block.
    state_commits: Vec<StateCommit<H>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct StateCommit<H> {
    state: SingleContractState,
    header: H,
}

impl<H: EvmBlockHeader> BlockHeaderCommit<H> for HistoryCommit<H> {
    fn commit(mut self, header: &Sealed<H>, config_id: B256) -> Commitment {
        let mut header = header.as_sealed_ref();

        for state_commit in &mut self.state_commits {
            let state_header = state_commit.header.seal_ref_slow();

            // verify that the block to query is in the allowed history window
            assert!(
                header.number() < state_header.number()
                    && state_header.number() - header.number()
                        <= execution_hash::HISTORY_SERVE_WINDOW,
                "Block outside of history range"
            );

            // verify that the state is valid with respect to the commitment header
            assert_eq!(
                &state_commit.state.root(),
                state_header.state_root(),
                "State root mismatch"
            );

            let execution_hash =
                ExecutionHashContract::get_from_db(&mut state_commit.state, header.number())
                    .unwrap();
            assert_eq!(execution_hash, header.seal(), "Execution hash mismatch");

            header = state_header;
        }

        Commitment::new(
            CommitmentVersion::Block as u16,
            header.number(),
            header.seal(),
            config_id,
        )
    }
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use alloy::{
        network::{BlockResponse, Network},
        providers::Provider,
    };
    use anyhow::{anyhow, ensure, Context};
    use std::{fmt::Display, iter};

    impl<H: EvmBlockHeader + Clone> HistoryCommit<H> {
        /// Creates a `HistoryCommit` from an EVM execution block header and a later commitment
        /// header.
        ///
        /// This method constructs a chain of proofs to link the `execution_header` to the
        /// `commitment_header` via the EIP-2935 execution hash contract.
        /// It effectively proves that the `execution_header` is an ancestor of a state verifiable
        /// by the `commitment_header`.
        pub(crate) async fn from_headers<N, P>(
            execution_header: &Sealed<H>,
            commitment_header: &Sealed<H>,
            rpc_provider: P,
        ) -> anyhow::Result<Self>
        where
            N: Network,
            P: Provider<N>,
            H: TryFrom<<N as Network>::HeaderResponse>,
            <H as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
        {
            ensure!(
                execution_header.number() < commitment_header.number(),
                "EVM execution block not before commitment block"
            );

            let mut current_state_header = execution_header.clone();

            let mut state_commits: Vec<StateCommit<H>> = Vec::new();
            for number in (execution_header.number() + execution_hash::HISTORY_SERVE_WINDOW
                ..commitment_header.number())
                .step_by(execution_hash::HISTORY_SERVE_WINDOW as usize)
                .chain(iter::once(commitment_header.number()))
            {
                let rpc_block = rpc_provider
                    .get_block_by_number(number.into())
                    .await
                    .context("eth_getBlockByNumber failed")?
                    .with_context(|| format!("block {number} not found"))?;

                let rpc_header = rpc_block.header().clone();
                let header: H = rpc_header
                    .try_into()
                    .map_err(|err| anyhow!("header invalid: {}", err))?;
                let header = header.seal_slow();

                let (hash, state_witness) = execution_hash::preflight_get(
                    current_state_header.number(),
                    &rpc_provider,
                    header.seal().into(),
                )
                .await
                .context("failed to preflight execution hash contract")?;
                ensure!(
                    current_state_header.seal() == hash,
                    "final block does not match the commitment block"
                );

                state_commits.push(StateCommit {
                    state: state_witness,
                    header: header.inner().clone(),
                });

                current_state_header = header;
            }
            ensure!(current_state_header.seal() == commitment_header.seal());

            log::debug!("Generated {} state commitments", state_commits.len());

            Ok(HistoryCommit { state_commits })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ethereum::EthBlockHeader, test_utils::get_el_url};
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy_eips::BlockNumberOrTag;
    use alloy_primitives::Sealable;
    use execution_hash::HISTORY_SERVE_WINDOW;
    use test_log::test;

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn create_and_check() {
        async fn check_dist(el: impl Provider, n: u64) -> anyhow::Result<()> {
            let latest_block = el
                .get_block_by_number(BlockNumberOrTag::Latest)
                .await?
                .unwrap();
            let latest_header: EthBlockHeader = latest_block.header.try_into()?;
            let latest_header = latest_header.seal_slow();

            let execution_block = el
                .get_block_by_number((latest_header.number() - n).into())
                .await?
                .unwrap();
            let execution_header: EthBlockHeader = execution_block.header.try_into()?;
            let execution_header = execution_header.seal_slow();

            let commit = HistoryCommit::from_headers(&execution_header, &latest_header, el).await?;

            let commitment = commit.commit(&execution_header, B256::default());
            assert_eq!(commitment.digest, latest_header.seal());

            Ok(())
        }

        let el = ProviderBuilder::default().connect_http(get_el_url());

        check_dist(&el, 1).await.unwrap();
        check_dist(&el, HISTORY_SERVE_WINDOW).await.unwrap();
        check_dist(&el, 20_000).await.unwrap();
    }
}
