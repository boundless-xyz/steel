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

//! Types related to commitments to a historical state relying on EIP-2935.

use alloy_primitives::{B256, Sealed, U256};
use history_storage::HistoryStorageContract;
use serde::{Deserialize, Serialize};

use crate::{
    BlockHeaderCommit, BlockInput, Commitment, CommitmentVersion, EvmBlockHeader, EvmFactory,
    EvmSpecId, GuestEvmEnv, config::ChainSpec, history::state::SingleContractState,
};

mod history_storage;

/// Input recursively committing to multiple execution block hashes relying on EIP-2935.
#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryInput<F: EvmFactory> {
    input: BlockInput<F>,
    commit: HistoryCommit<<F as EvmFactory>::Header>,
}

/// Commitment that an execution block is an ancestor of a specific other execution block.
///
/// This struct encapsulates the necessary data to prove that a given execution block is part of the
/// canonical chain.
#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryCommit<H> {
    /// Iterative commits for verifying the execution block as an ancestor of some other block.
    state_commits: Vec<StateCommit<H>>,
}

/// Represents a commitment of a history storage contract state to the corresponding block hash.
#[derive(Clone, Serialize, Deserialize)]
struct StateCommit<H> {
    /// State for verifying the previous execution block hash.
    state: SingleContractState,
    /// Header belonging to the state.
    header: H,
}

impl<F: EvmFactory> HistoryInput<F> {
    /// Creates a new input from a [BlockInput] and a [HistoryCommit].
    pub const fn new(
        input: BlockInput<F>,
        commit: HistoryCommit<<F as EvmFactory>::Header>,
    ) -> Self {
        Self { input, commit }
    }

    /// Converts the input into a [EvmEnv] for verifiable state access in the guest.
    ///
    /// [EvmEnv]: crate::EvmEnv
    pub fn into_env(self, chain_spec: &ChainSpec<F::SpecId>) -> GuestEvmEnv<F> {
        let mut env = self.input.into_env(chain_spec);

        // It is sufficient to check that the EVM block supports EIP-2935, as this guarantees that
        // the history storage contract is supported and that its hash is included.
        // Without this check, there is a theoretical risk of a malicious contract being deployed
        // with the same code at the same address on chains without EIP-2935 support.
        assert!(env.spec_id.has_eip2935(), "EIP-2935 required");
        env.commit = self.commit.commit(&env.header, env.commit.configID);

        env
    }
}

impl<H: EvmBlockHeader> BlockHeaderCommit<H> for HistoryCommit<H> {
    /// Generates a commitment that proves the given block header is part of the chain history.
    ///
    /// Panics if the provided [HistoryCommit] data is invalid or inconsistent.
    fn commit(mut self, header: &Sealed<H>, config_id: B256) -> Commitment {
        let mut header = header.as_sealed_ref();

        // starting from header, "walk forward" along state_commits to reach a later execution hash
        for state_commit in &mut self.state_commits {
            let state_header = state_commit.header.seal_ref_slow();

            // verify that the block to query is in the allowed history window
            assert!(
                header.number() < state_header.number()
                    && state_header.number() - header.number()
                        <= history_storage::HISTORY_SERVE_WINDOW.to(),
                "Block outside of EIP-2935 history range"
            );

            // verify that the state is valid with respect to the commitment header
            assert_eq!(
                &state_commit.state.root(),
                state_header.state_root(),
                "State root mismatch"
            );

            let block_number = U256::from(header.number());
            let execution_hash = HistoryStorageContract::new(&mut state_commit.state)
                .and_then(|mut c| c.get_unchecked(block_number))
                .expect("History storage contract failed");
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
    use crate::host::db::ProviderDb;
    use alloy::{
        network::{BlockResponse, Network},
        providers::Provider,
    };
    use alloy_primitives::BlockNumber;
    use anyhow::{Context, anyhow, ensure};
    use std::{fmt::Display, iter};

    impl<H: EvmBlockHeader> HistoryCommit<H> {
        /// Creates a `HistoryCommit` from an EVM execution block header and a later commitment
        /// header.
        ///
        /// This method constructs a chain of proofs to link the `execution_header` to the
        /// `commitment_header` via the EIP-2935 history storage contract.
        /// It effectively proves that the `execution_header` is an ancestor of a state verifiable
        /// by the `commitment_header`.
        pub(crate) async fn from_headers<P, N>(
            execution_header: &Sealed<H>,
            commitment_header: &Sealed<H>,
            rpc_provider: P,
        ) -> anyhow::Result<Self>
        where
            N: Network,
            P: Provider<N>,
            H: Clone + TryFrom<<N as Network>::HeaderResponse>,
            <H as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
        {
            ensure!(
                execution_header.number() < commitment_header.number(),
                "EVM execution block not before commitment block"
            );

            let mut current_state_header = execution_header.clone();

            let mut state_commits: Vec<StateCommit<H>> = Vec::new();
            for number in (execution_header.number()
                + history_storage::HISTORY_SERVE_WINDOW.to::<BlockNumber>()
                ..commitment_header.number())
                .step_by(history_storage::HISTORY_SERVE_WINDOW.to())
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
                    .map_err(|err| anyhow!("header invalid: {err}"))?;
                let header = header.seal_slow();
                let db = ProviderDb::new(&rpc_provider, Default::default(), header.seal());

                let block_number = U256::from(current_state_header.number());
                let (hash, state_witness) = HistoryStorageContract::preflight(db)
                    .get_unchecked(block_number)
                    .await
                    .context("failed to preflight history storage contract")?;
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
    use history_storage::HISTORY_SERVE_WINDOW;
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
        check_dist(&el, HISTORY_SERVE_WINDOW.to()).await.unwrap();
        check_dist(&el, 20_000).await.unwrap();
    }
}
