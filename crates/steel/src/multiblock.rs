// Copyright 2026 RISC Zero, Inc.
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

//! Types for verifiable computation across multiple blocks of the same chain.
//!
//! This module provides [MultiblockEvmEnv] and [MultiblockEvmInput] for executing and proving EVM
//! state queries spanning multiple historical blocks. The guest environment validates that all
//! blocks belong to the same chain by verifying commitments between consecutive blocks.
use crate::{
    Commitment, EvmBlockHeader, EvmEnv, EvmFactory, EvmInput, GuestEvmEnv, StateDb, SteelVerifier,
    config::ChainSpec,
};
use alloy_primitives::BlockNumber;
use delegate::delegate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A sequence of [EvmEnv] that form a subsequence in a single chain.
///
/// ### Examples
///
/// Query token balances across multiple blocks to compute a time-weighted average:
///
/// ```rust,no_run
/// # use risc0_steel::{
/// #    ethereum::{EthEvmEnv, ETH_MAINNET_CHAIN_SPEC},
/// #    EvmBlockHeader
/// # };
/// # use alloy::providers::{Provider,ProviderBuilder};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> anyhow::Result<()> {
/// // === Host Setup ===
/// let provider =
///     ProviderBuilder::new().connect_http("https://ethereum-rpc.publicnode.com".parse()?);
/// let latest = provider.get_block_number().await?;
///
/// let builder = EthEvmEnv::builder().provider(provider).chain_spec(&ETH_MAINNET_CHAIN_SPEC);
/// let mut envs = builder.build_multi();
///
/// // Query state at multiple points in time (e.g., every ~24 hours)
/// for i in 0..3 {
///     let block = latest - i * 7200; // ~24h apart at 12s/block
///     let env = envs.get_or_build(block).await?;
///     // Preflight your contract calls here...
/// }
///
/// // Generate input for the guest
/// let evm_input = envs.into_input().await?;
///
/// // === Guest Execution ===
/// let envs = evm_input.into_env(&ETH_MAINNET_CHAIN_SPEC);
///
/// // Process each block's state
/// for env in envs.iter() {
///     let block_number = env.header().number();
///     // Execute contract calls...
/// }
///
/// // Commit using the final block's commitment (covers all blocks)
/// let commitment = envs.into_commitment();
/// # Ok(())
/// # }
/// ```
pub struct MultiblockEvmEnv<D, F: EvmFactory, C>(BTreeMap<BlockNumber, EvmEnv<D, F, C>>);

/// The serializable input to derive and validate a [MultiblockEvmEnv] from.
#[derive(Clone, Serialize, Deserialize)]
pub struct MultiblockEvmInput<F: EvmFactory>(Vec<EvmInput<F>>);

impl<F: EvmFactory> MultiblockEvmInput<F> {
    /// Converts the input into a [MultiblockEvmEnv] for verifiable state access in the guest.
    ///
    /// This method verifies that all the envs belong to the same chain and panics if not.
    pub fn into_env(
        self,
        chain_spec: &ChainSpec<F::SpecId>,
    ) -> MultiblockEvmEnv<StateDb, F, Commitment> {
        assert!(!self.0.is_empty(), "Empty environment");

        let mut envs = BTreeMap::new();
        for env_input in self.0 {
            let env = env_input.into_env(chain_spec);
            if let Some(collision) = envs.insert(env.header().number(), env) {
                panic!(
                    "More than one env for block {}",
                    collision.header().number()
                );
            };
        }

        let mut prev_commit: Option<&Commitment> = None;
        for env in envs.values() {
            if let Some(commit) = prev_commit {
                SteelVerifier::new(env).verify(commit);
            }
            prev_commit = Some(env.commitment());
        }

        MultiblockEvmEnv(envs)
    }
}

impl<F: EvmFactory> MultiblockEvmEnv<StateDb, F, Commitment> {
    delegate! {
        to self.0 {
            /// Returns the number of environments.
            pub fn len(&self) -> usize;
            /// Returns `true` if it contains no environments.
            pub fn is_empty(&self) -> bool;
        }
    }

    /// Returns a reference to the environment corresponding to the block number.
    pub fn get(&self, num: BlockNumber) -> Option<&GuestEvmEnv<F>> {
        self.0.get(&num)
    }

    /// Returns the first environment, i.e. the environment with the smallest block number.
    pub fn first(&self) -> &GuestEvmEnv<F> {
        // safe unwrap: MultiblockEvmEnv<StateDb, F, Commitment> cannot be constructed empty
        self.0.first_key_value().unwrap().1
    }

    /// Returns the final environment, i.e. the environment with the largest block number.
    pub fn last(&self) -> &GuestEvmEnv<F> {
        // safe unwrap: MultiblockEvmEnv<StateDb, F, Commitment> cannot be constructed empty
        self.0.last_key_value().unwrap().1
    }

    /// Gets an iterator over the block numbers in ascending order.
    pub fn block_numbers(&self) -> impl Iterator<Item = BlockNumber> + '_ {
        self.0.keys().copied()
    }

    /// Gets an iterator over the environments in ascending block number order.
    pub fn iter(&self) -> impl Iterator<Item = &GuestEvmEnv<F>> {
        self.0.values()
    }

    /// Returns the [Commitment] for the entire block sequence.
    ///
    /// The returned commitment is from the final (highest) block. Verifying this single commitment
    /// on-chain validates the entire sequence because each block is cryptographically linked to its
    /// predecessors.
    #[must_use]
    pub fn commitment(&self) -> &Commitment {
        self.last().commitment()
    }

    /// Consumes and returns the [Commitment] for the entire block sequence.
    ///
    /// The returned commitment is from the final (highest) block. Verifying this single commitment
    /// on-chain validates the entire sequence because each block is cryptographically linked to its
    /// predecessors.
    #[must_use]
    pub fn into_commitment(self) -> Commitment {
        self.commitment().clone()
    }
}

#[cfg(feature = "host")]
pub(crate) mod host {
    use super::*;
    use crate::{
        EvmSpecId,
        host::{
            EvmEnvBuilder, HostCommit, HostEvmEnv, InputBuilder,
            db::{ProofDb, ProviderDb},
        },
        verifier,
    };
    use alloy::providers::{Network, Provider};
    use anyhow::{Context, bail, ensure};
    use delegate::delegate;
    use std::{collections::btree_map::Entry, fmt::Display};

    /// A host-side collection of [EvmEnv] instances spanning multiple blocks of the same chain.
    ///
    /// This type uses an [EvmEnvBuilder] as a template to construct individual environments on
    /// demand via [HostMultiblockEvmEnv::get_or_build()]. This design avoids duplicating
    /// configuration methods while ensuring all environments share consistent settings (provider,
    /// chain spec, commitment type).
    ///
    /// See [MultiblockEvmEnv] for usage examples.
    pub struct HostMultiblockEvmEnv<'a, N, P, F: EvmFactory, C> {
        template: EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, C>,
        env: MultiblockEvmEnv<ProofDb<ProviderDb<N, P>>, F, HostCommit<()>>,
    }

    impl<'a, N, P, F, C> HostMultiblockEvmEnv<'a, N, P, F, C>
    where
        N: Network,
        P: Provider<N> + Clone + 'static,
        F: EvmFactory,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
        F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
        <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, C>: InputBuilder<N, P, F>,
    {
        /// Creates a new [HostMultiblockEvmEnv] using the given [EvmEnvBuilder] as a template.
        ///
        /// Prefer using [EvmEnvBuilder::build_multi()] for a more fluent API.
        ///
        /// Any execution block configured on the template (e.g. via [EvmEnvBuilder::block_number])
        /// is ignored; blocks are selected via [HostMultiblockEvmEnv::get_or_build].
        pub fn from_builder(template: EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, C>) -> Self {
            if template.has_explicit_block() {
                log::warn!(
                    "the execution block configured on the builder is ignored by build_multi; \
                    select blocks via get_or_build()"
                );
            }
            Self {
                template,
                env: MultiblockEvmEnv(BTreeMap::new()),
            }
        }

        delegate! {
            to self.env.0 {
                 /// Returns the number of environments.
                pub fn len(&self) -> usize;
                /// Returns `true` if it contains no environments.
                pub fn is_empty(&self) -> bool;
            }
        }

        /// Gets or creates an environment for the specified block number.
        ///
        /// If an environment for this block already exists, returns a mutable reference to it.
        /// Otherwise, creates a new environment using the template builder and inserts it.
        ///
        /// Blocks can be added in any order, they will be properly ordered when
        /// [HostMultiblockEvmEnv::into_input] is called.
        ///
        /// When the template is configured with an explicit commitment target (e.g. via
        /// [EvmEnvBuilder::commitment_block_number]), every added block must be strictly before
        /// that target; passing a block at or after it returns an error.
        ///
        /// With a default beacon commitment (from [EvmEnvBuilder::beacon_api]), the largest added
        /// block must not be the chain head, as the commitment requires the subsequent block; use
        /// `latest - 1` or a safe/finalized block. This is only detected when
        /// [HostMultiblockEvmEnv::into_input] is called.
        pub async fn get_or_build(
            &mut self,
            num: BlockNumber,
        ) -> anyhow::Result<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            // reject blocks that cannot precede the commitment target before any RPC work; targets
            // given as a hash, tag, or slot are only checked when building the final input
            if let Some(target) = self.template.commitment_target_number() {
                ensure!(
                    num < target,
                    "block {num} is not before the commitment target block {target}"
                );
            }
            match self.env.0.entry(num) {
                Entry::Occupied(entry) => Ok(entry.into_mut()),
                Entry::Vacant(entry) => {
                    Ok(entry.insert(self.template.clone_with_block(num).build().await?))
                }
            }
        }

        /// Returns a mutable reference to the environment corresponding to the block number.
        pub fn get_mut(
            &mut self,
            num: BlockNumber,
        ) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.get_mut(&num)
        }

        /// Returns a mutable reference to the first environment, i.e. the environment with the
        /// smallest block number, or `None` if empty.
        pub fn first_mut(&mut self) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.first_entry().map(|entry| entry.into_mut())
        }

        /// Returns a mutable reference to the last environment, i.e. the environment with the
        /// largest block number, or `None` if empty.
        pub fn last_mut(&mut self) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.last_entry().map(|entry| entry.into_mut())
        }

        /// Gets an iterator over the block numbers in ascending order.
        pub fn block_numbers(&self) -> impl Iterator<Item = BlockNumber> + '_ {
            self.env.0.keys().copied()
        }

        /// Gets a mutable iterator over the environments in order by their block number.
        pub fn iter_mut(
            &mut self,
        ) -> impl Iterator<Item = &mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.values_mut()
        }

        /// Converts the environment into a [MultiblockEvmInput] using the commitment method
        /// specified in the template builder.
        ///
        /// Each environment's commitment is verified against its successor using [SteelVerifier],
        /// ensuring all blocks belong to the same chain. The verification strategy adapts based on
        /// distance between blocks (direct block hash vs. EIP-2935 history commitment).
        ///
        /// A single block is supported; in that case no inter-block verification is performed.
        pub async fn into_input(self) -> anyhow::Result<MultiblockEvmInput<F>> {
            ensure!(
                !self.is_empty(),
                "cannot build input: no blocks added via get_or_build()"
            );

            let mut inputs = Vec::with_capacity(self.env.0.len());
            let mut iter = self.env.0.into_values();
            // safe unwrap: empty checked above
            let mut current_env = iter.next().unwrap();

            for mut next_env in iter {
                let current_block = current_env.header().number();
                let next_block = next_env.header().number();
                let dist = next_block - current_block;

                // check current_env, not next_env: EIP-2935 storage is not backfilled at
                // activation, so a pre-fork block's hash is never retrievable from it
                let input = if dist > verifier::HISTORY_LIMIT && !current_env.spec_id.has_eip2935()
                {
                    bail!(
                        "EIP-2935 required: distance between blocks \
                        {current_block} and {next_block} exceeds BLOCKHASH limit"
                    )
                } else if dist <= verifier::EIP2935_HISTORY_LIMIT {
                    // Short-range: direct block commitment
                    let commit = current_env.commitment();
                    SteelVerifier::preflight(&mut next_env)
                        .verify(&commit)
                        .await
                        .with_context(|| {
                            format!("block {current_block}: failed to verify {commit}")
                        })?;
                    current_env.into_input().await
                } else {
                    // Long-range: intermediate EIP-2935 history commitment
                    let target_block = next_block - verifier::EIP2935_HISTORY_LIMIT;
                    let builder = self
                        .template
                        .clone_with_block(current_env.header().seal())
                        .commitment_block_number(target_block);
                    // build a new env carrying the history commitment, then merge in current_env's
                    // data; merge_state keeps the new commitment and absorbs the state
                    let history_env = builder.build().await.with_context(|| {
                        format!("block {current_block}: failed to build EIP-2935 history commitment to {target_block}")
                    })?.merge_state(current_env)?;

                    let commit = history_env.commitment();
                    SteelVerifier::preflight(&mut next_env)
                        .verify(&commit)
                        .await
                        .with_context(|| {
                            format!("block {current_block}: failed to verify {commit}")
                        })?;
                    history_env.into_input().await
                }
                .with_context(|| format!("failed to build input for block {current_block}"))?;

                inputs.push(input);
                current_env = next_env;
            }

            // Final environment: use the template's commitment type
            let final_block = current_env.header().number();
            let final_input = self
                .template
                .build_input(current_env)
                .await
                .with_context(|| format!("failed to build input for final block {final_block}"))?;
            inputs.push(final_input);

            Ok(MultiblockEvmInput(inputs))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Account, CommitmentVersion,
        ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv},
        host::HostMultiblockEvmEnv,
        test_utils::{get_cl_url, get_el_url},
    };
    use alloy::{
        network::TransactionBuilder,
        node_bindings::Anvil,
        providers::{Provider, ProviderBuilder},
    };
    use alloy_consensus::BlockHeader;
    use alloy_primitives::{Address, U256, address};
    use alloy_rpc_types::{BlockId, TransactionRequest};
    use revm::primitives::hardfork::SpecId;
    use test_log::test;

    #[test(tokio::test)]
    async fn successive_blocks() -> anyhow::Result<()> {
        const N: u64 = 5;
        const ADDRESS: Address = address!("0x0000000000000000000000000000000000000042");

        let chain_spec = ChainSpec::new_single(31337, SpecId::CANCUN);
        let provider = ProviderBuilder::new().connect_anvil_with_config(Anvil::cancun);

        let sender = provider.get_accounts().await?[0];
        for _ in 0..N {
            let tx = TransactionRequest::default()
                .with_from(sender)
                .with_to(ADDRESS)
                .with_value(U256::from(1));
            provider.send_transaction(tx).await?.watch().await?;
        }

        let block_hash = provider
            .get_block(BlockId::default())
            .await?
            .unwrap()
            .hash();

        let builder = EthEvmEnv::builder()
            .provider(provider)
            .chain_spec(&chain_spec);
        let mut host_env = HostMultiblockEvmEnv::from_builder(builder);

        for i in 1..=N {
            let env = host_env.get_or_build(i as BlockNumber).await?;
            Account::preflight(ADDRESS, env).info().await?;
        }

        let input = host_env.into_input().await?;

        let guest_env = input.into_env(&chain_spec);
        for i in 1..=N {
            let info = Account::new(ADDRESS, guest_env.get(i).unwrap()).info();
            assert_eq!(info.balance, U256::from(i));
        }

        let commitment = dbg!(guest_env.into_commitment());
        assert_eq!(commitment.decode_id().1, CommitmentVersion::Block as u16);
        assert_eq!(commitment.digest, block_hash);
        assert_eq!(commitment.configID, chain_spec.digest());

        Ok(())
    }

    #[test(tokio::test)]
    async fn get_or_build_rejects_block_at_or_after_commitment_target() -> anyhow::Result<()> {
        let chain_spec = ChainSpec::new_single(31337, SpecId::PRAGUE);
        let provider = ProviderBuilder::new().connect_anvil_with_config(Anvil::cancun);

        let builder = EthEvmEnv::builder()
            .provider(provider)
            .chain_spec(&chain_spec)
            .commitment_block_number(100);
        let mut host_env = HostMultiblockEvmEnv::from_builder(builder);

        // a block at or after the target is rejected before any RPC work
        for block in [100, 101] {
            let err = host_env
                .get_or_build(block)
                .await
                .err()
                .expect("expected an error");
            assert!(
                err.to_string()
                    .contains("is not before the commitment target"),
                "{err:#}"
            );
        }

        Ok(())
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn eip2935_history_commitment() -> anyhow::Result<()> {
        const N: u64 = 3;
        // TODO(https://github.com/foundry-rs/foundry/issues/10357): Use Anvil provider
        let provider = ProviderBuilder::new().connect_http(get_el_url());

        let block_number = provider.get_block_number().await?;
        let block_hash = provider
            .get_block_by_number(block_number.into())
            .await?
            .unwrap()
            .hash();

        let builder = EthEvmEnv::builder()
            .provider(provider)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
            .commitment_block_hash(block_hash);
        let mut host_env = HostMultiblockEvmEnv::from_builder(builder);

        for i in 1..=N {
            let env = host_env.get_or_build(block_number - i * 8192).await?;
            Account::preflight(Address::ZERO, env).info().await?;
        }

        let input = host_env.into_input().await?;
        let commitment = dbg!(input.into_env(&ETH_MAINNET_CHAIN_SPEC).into_commitment());
        assert_eq!(commitment.decode_id().1, CommitmentVersion::Block as u16);
        assert_eq!(commitment.digest, block_hash);

        Ok(())
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn beacon_commitment() -> anyhow::Result<()> {
        let el = ProviderBuilder::new().connect_http(get_el_url());

        let latest = el.get_block_number().await?;
        let parent_beacon_block_root = el
            .get_block_by_number(latest.into())
            .await?
            .unwrap()
            .header
            .parent_beacon_block_root()
            .unwrap();

        let builder = EthEvmEnv::builder()
            .provider(el)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
            .beacon_api(get_cl_url());
        let mut host_env = HostMultiblockEvmEnv::from_builder(builder);

        let env = host_env.get_or_build(latest - 1).await?;
        Account::preflight(Address::ZERO, env).info().await?;

        let input = host_env.into_input().await?;
        let commitment = dbg!(input.into_env(&ETH_MAINNET_CHAIN_SPEC).into_commitment());
        assert_eq!(commitment.decode_id().1, CommitmentVersion::Beacon as u16);
        assert_eq!(commitment.digest, parent_beacon_block_root);

        Ok(())
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn history_commitment() -> anyhow::Result<()> {
        let el = ProviderBuilder::new().connect_http(get_el_url());

        let latest = el.get_block_number().await?;
        let parent_beacon_block_root = el
            .get_block_by_number(latest.into())
            .await?
            .unwrap()
            .header
            .parent_beacon_block_root()
            .unwrap();

        let builder = EthEvmEnv::builder()
            .provider(el)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
            .beacon_api(get_cl_url())
            .commitment_block_number(latest - 1);
        let mut host_env = HostMultiblockEvmEnv::from_builder(builder);

        let env = host_env.get_or_build(latest - 2).await?;
        Account::preflight(Address::ZERO, env).info().await?;

        let input = host_env.into_input().await?;
        let commitment = dbg!(input.into_env(&ETH_MAINNET_CHAIN_SPEC).into_commitment());
        assert_eq!(commitment.decode_id().1, CommitmentVersion::Beacon as u16);
        assert_eq!(commitment.digest, parent_beacon_block_root);

        Ok(())
    }
}
