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

//! Types related to queries and environments over multiple blocks of the same chain.
use crate::{
    config::ChainSpec, Commitment, EvmBlockHeader, EvmEnv, EvmFactory, EvmInput, GuestEvmEnv,
    StateDb, SteelVerifier,
};
use alloy_primitives::BlockNumber;
use delegate::delegate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A sequence of [EvmEnv] that form a subsequence in a single chain.
///
/// ### Examples
///
/// ```rust
/// # use risc0_steel::{
/// #    ethereum::{EthEvmInput, EthEvmEnv, ETH_MAINNET_CHAIN_SPEC},
/// #    host::{BlockNumberOrTag, HostMultiblockEvmEnv}
/// # };
/// # use alloy::providers::{ext::AnvilApi, ProviderBuilder};
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> anyhow::Result<()> {
/// // === Host Setup ===
/// # let p = ProviderBuilder::new().connect_anvil();
/// # p.anvil_mine(Some(3), None).await?;
/// let builder = EthEvmEnv::builder().provider(p).chain_spec(&ETH_MAINNET_CHAIN_SPEC);
/// // create multiblock environment from regular builder
/// let mut envs = HostMultiblockEvmEnv::from_builder(builder);
///
/// // get for block in the chain
/// let host_env = envs.get_or_build(1).await?;
/// // use like regular EthEvmEnv
/// // ... get for more blocks
/// let host_env = envs.get_or_build(2).await?;
///
/// // generate input for the guest
/// let evm_input = envs.into_input().await?;
///
/// // === Guest Setup & Execution ===
/// let envs = evm_input.into_env(&ETH_MAINNET_CHAIN_SPEC);
///
/// // execute the same queries on the same blocks in the guest
/// let guest_env = envs.get(1).unwrap();
/// let guest_env = envs.get(2).unwrap();
///
/// // get commitment for all the environments
/// let commit = envs.into_commitment();
/// # Ok(())
/// # }
/// ```
pub struct MultiblockEvmEnv<D, F: EvmFactory, C>(BTreeMap<BlockNumber, EvmEnv<D, F, C>>);

/// The serializable input to derive and validate an [MultiblockEvmInput] from.
#[derive(Clone, Serialize, Deserialize)]
pub struct MultiblockEvmInput<F: EvmFactory>(Vec<EvmInput<F>>);

impl<F: EvmFactory> MultiblockEvmInput<F> {
    /// Converts the input into a [MultiblockEvmEnv] for verifiable state access in the guest.
    ///
    /// This method verifies that all the envs belong to the same chain.
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

        for (env_prev, env) in envs.values().zip(envs.values().skip(1)) {
            SteelVerifier::new(env).verify(env_prev.commitment());
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

    /// Returns the final environment, i.e. the environment with the largest block number.
    pub fn last(&self) -> &GuestEvmEnv<F> {
        // safe unwrap: MultiblockEvmEnv<StateDb, F, Commitment> cannot be constructed empty
        self.0.last_key_value().unwrap().1
    }

    /// Gets an iterator over the environments in order by their block number ascending.
    pub fn iter(&self) -> impl Iterator<Item = &GuestEvmEnv<F>> {
        self.0.values()
    }

    /// Returns the [Commitment] used to validate the entire chain of environments.
    pub fn commitment(&self) -> &Commitment {
        self.last().commitment()
    }

    /// Consumes and returns the [Commitment] used to validate the entire chain of environments.
    pub fn into_commitment(mut self) -> Commitment {
        // safe unwrap: MultiblockEvmEnv<StateDb, F, Commitment> cannot be constructed empty
        let env = self.0.pop_last().unwrap().1;
        env.into_commitment()
    }
}

#[cfg(feature = "host")]
pub(crate) mod host {
    use super::*;
    use crate::{
        host::{
            db::{ProofDb, ProviderDb},
            EvmEnvBuilder, HostCommit, HostEvmEnv, InputBuilder,
        },
        verifier, EvmSpecId,
    };
    use alloy::providers::{Network, Provider};
    use anyhow::{bail, ensure, Context};
    use delegate::delegate;
    use std::{collections::btree_map::Entry, fmt::Display};

    /// A sequence of [EvmEnv] that form a subsequence in a single chain.
    ///
    /// See [MultiblockEvmEnv] for usage examples.
    pub struct HostMultiblockEvmEnv<'a, N, P, F: EvmFactory, B> {
        builder: EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, B>,
        env: MultiblockEvmEnv<ProofDb<ProviderDb<N, P>>, F, HostCommit<()>>,
    }

    #[allow(private_bounds)]
    impl<'a, N, P, F, B> HostMultiblockEvmEnv<'a, N, P, F, B>
    where
        N: Network,
        P: Provider<N> + Clone + 'static,
        F: EvmFactory,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
        F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
        <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, B>: InputBuilder<N, P, F>,
    {
        /// Creates a new [HostMultiblockEvmEnv] from the given [EvmEnvBuilder].
        ///
        /// This ignores any potential EVM execution block set in the builder, but all other options
        /// specified with this builder are incorporated when creating the individual [EvmEnv].
        pub fn from_builder(builder: EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, B>) -> Self {
            Self {
                builder,
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

        /// Ensures an environment is in the [HostMultiblockEvmEnv] by using the provided builder to
        /// create an environment if empty. It then returns a mutable reference to the environment.
        pub async fn get_or_build(
            &mut self,
            num: BlockNumber,
        ) -> anyhow::Result<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            match self.env.0.entry(num) {
                Entry::Occupied(entry) => Ok(entry.into_mut()),
                Entry::Vacant(entry) => Ok(entry.insert(self.builder.to_block(num).build().await?)),
            }
        }

        /// Returns a mutable reference to the environment corresponding to the block number.
        pub fn get_mut(
            &mut self,
            num: BlockNumber,
        ) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.get_mut(&num)
        }

        /// Returns a mutable reference to the final environment, i.e. the environment with the
        /// largest block number.
        pub fn last_mut(&mut self) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.last_entry().map(|entry| entry.into_mut())
        }

        /// Gets a mutable iterator over the environments in order by their block number.
        pub fn iter_mut(
            &mut self,
        ) -> impl Iterator<Item = &mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.values_mut()
        }

        /// Converts the environment into a [MultiblockEvmInput] using the commitment method which
        /// was specified in the builder during creation.
        ///
        /// This method uses [SteelVerifier] internally to link the individual [EvmEnv].
        pub async fn into_input(self) -> anyhow::Result<MultiblockEvmInput<F>> {
            ensure!(!self.is_empty(), "environment must not be empty");

            let mut inputs = Vec::with_capacity(self.env.0.len());

            let mut iter = self.env.0.into_values().peekable();
            while let Some(env) = iter.next() {
                let number = env.header().number();
                match iter.peek_mut() {
                    Some(next_env) => {
                        let dist = next_env.header.number() - number;
                        // decide verification strategy based on distance and spec support
                        let input = if dist > verifier::HISTORY_LIMIT && !env.spec_id.has_eip2935()
                        {
                            bail!(
                                "EIP-2935 required: \
                                block distance {dist} exceeds BLOCKHASH history limit"
                            )
                        } else if dist <= verifier::EIP2935_HISTORY_LIMIT {
                            // short-range: verify directly using standard block commitment
                            let commit = env.commitment();
                            SteelVerifier::preflight(next_env)
                                .verify(&commit)
                                .await
                                .with_context(|| format!("failed to verify: {commit}"))?;
                            env.into_input().await
                        } else {
                            // long‑range: use an intermediate EIP‑2935 history commit
                            let target = next_env.header.number() - verifier::EIP2935_HISTORY_LIMIT;
                            let builder = self
                                .builder
                                .to_block(env.header().seal())
                                .commitment_block_number(target);
                            let env = builder.build().await?.merge(env)?;

                            let commit = env.commitment();
                            SteelVerifier::preflight(next_env)
                                .verify(&commit)
                                .await
                                .with_context(|| format!("failed to verify: {commit}"))?;
                            env.into_input().await
                        }
                        .with_context(|| format!("failed to build input for block {number}"))?;
                        inputs.push(input);
                    }
                    None => {
                        // if there is no next env, we are processing the final env and can return
                        let input = self.builder.build_input(env).await.with_context(|| {
                            format!("failed to build final input for block {number}")
                        })?;
                        inputs.push(input);

                        return Ok(MultiblockEvmInput(inputs));
                    }
                }
            }

            unreachable!()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ethereum::{EthEvmEnv, ETH_MAINNET_CHAIN_SPEC},
        host::HostMultiblockEvmEnv,
        test_utils::{get_cl_url, get_el_url},
        Account, CommitmentVersion,
    };
    use alloy::{
        network::TransactionBuilder,
        node_bindings::Anvil,
        providers::{Provider, ProviderBuilder},
    };
    use alloy_consensus::BlockHeader;
    use alloy_primitives::{address, Address, U256};
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
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn eip2935_history_commitment() -> anyhow::Result<()> {
        const N: u64 = 3;
        // TODO: Make this an Anvil provider, once Anvil has EIP-2935 support
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
