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
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An ordered map of block numbers to [EvmEnv] that form a subsequence in a single chain.
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
                    "more than one env for block {}",
                    collision.header().number()
                );
            };
        }

        envs.values().reduce(|env_prev, env| {
            SteelVerifier::new(env).verify(env_prev.commitment());
            env
        });

        MultiblockEvmEnv(envs)
    }
}

impl<F: EvmFactory> MultiblockEvmEnv<StateDb, F, Commitment> {
    /// Returns a reference to the environment corresponding to the block number.
    pub fn get(&self, num: BlockNumber) -> Option<&GuestEvmEnv<F>> {
        self.0.get(&num)
    }

    /// Gets an iterator over the environments in order by their block number.
    pub fn iter(&self) -> impl Iterator<Item = &GuestEvmEnv<F>> {
        self.0.values()
    }

    /// Returns the [Commitment] used to validate the environment.
    pub fn commitment(&self) -> &Commitment {
        // safe unwrap: MultiblockEvmEnv<StateDb, F, Commitment> cannot be constructed empty
        let env = self.0.last_key_value().unwrap().1;
        env.commitment()
    }

    /// Consumes and returns the [Commitment] used to validate the environment.
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
            EvmEnvBuilder, HostCommit, HostEvmEnv,
        },
        verifier, EvmSpecId,
    };
    use alloy::providers::{Network, Provider};
    use anyhow::{bail, ensure};
    use private::InputBuilder;
    use std::{collections::btree_map::Entry, fmt::Display};

    mod private {
        use super::*;
        use crate::{
            ethereum::EthEvmFactory,
            host::{Beacon, Eip2935History},
        };
        use alloy::network::Ethereum;

        /// A private trait to handle the creation of different `EvmInput` variants from a generic
        /// `HostEvmEnv`.
        ///
        /// ### Design Rationale
        /// This pattern is an internal implementation detail used to manage the complexity arising
        /// from supporting multiple commitment types (`Block`, `Beacon`, `Eip2935History`, etc.).
        ///
        /// The primary goal is to avoid providing multiple implementations of the `into_input`
        /// method for all the commitment types. Such an approach would lead to significant code
        /// duplication and become difficult to maintain as new commitment types are added.
        /// While this pattern requires `#[allow(private_bounds)]` on a public method that use it,
        /// the benefit of improved code structure and maintainability is a worthwhile trade-off
        /// for this internal abstraction.
        pub(super) trait InputBuilder<D, F: EvmFactory> {
            async fn build_input(&self, env: HostEvmEnv<D, F, ()>) -> anyhow::Result<EvmInput<F>>;
        }

        impl<N, P, F: EvmFactory> InputBuilder<ProviderDb<N, P>, F>
            for EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, ()>
        where
            N: Network,
            P: Provider<N>,
            F::Header: TryFrom<<N as Network>::HeaderResponse>,
            <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
            F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
            <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        {
            async fn build_input(
                &self,
                env: HostEvmEnv<ProviderDb<N, P>, F, ()>,
            ) -> anyhow::Result<EvmInput<F>> {
                // ignore the builder and use the available env with block commitment directly
                env.into_input().await
            }
        }

        impl<N, P, F: EvmFactory> InputBuilder<ProviderDb<N, P>, F>
            for EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, Eip2935History>
        where
            N: Network,
            P: Provider<N> + Clone,
            F::Header: TryFrom<<N as Network>::HeaderResponse>,
            <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
            F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
            <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        {
            async fn build_input(
                &self,
                env: HostEvmEnv<ProviderDb<N, P>, F, ()>,
            ) -> anyhow::Result<EvmInput<F>> {
                let builder = self.clone().block_hash(env.header().seal());
                builder.build().await?.merge(env)?.into_input().await
            }
        }

        impl<P: Provider<Ethereum> + Clone> InputBuilder<ProviderDb<Ethereum, P>, EthEvmFactory>
            for EvmEnvBuilder<
                P,
                EthEvmFactory,
                &ChainSpec<<EthEvmFactory as EvmFactory>::SpecId>,
                Beacon,
            >
        {
            async fn build_input(
                &self,
                env: HostEvmEnv<ProviderDb<Ethereum, P>, EthEvmFactory, ()>,
            ) -> anyhow::Result<EvmInput<EthEvmFactory>> {
                let builder = self.clone().block_hash(env.header().seal());
                builder.build().await?.merge(env)?.into_input().await
            }
        }
    }

    impl<F: EvmFactory> MultiblockEvmEnv<(), F, ()> {
        /// Creates a builder for building a multiblock environment.
        pub fn builder() -> EvmEnvBuilder<(), F, (), ()> {
            EvmEnvBuilder::new()
        }
    }

    pub struct HostMultiblockEvmEnv<'a, N, P, F: EvmFactory, B> {
        builder: EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, B>,
        env: MultiblockEvmEnv<ProofDb<ProviderDb<N, P>>, F, HostCommit<()>>,
    }

    #[allow(private_bounds)]
    impl<'a, N, P, F, B> HostMultiblockEvmEnv<'a, N, P, F, B>
    where
        N: Network,
        P: Provider<N> + Clone + Send + Sync + 'static,
        F: EvmFactory,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
        F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
        <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        EvmEnvBuilder<P, F, &'a ChainSpec<F::SpecId>, B>: InputBuilder<ProviderDb<N, P>, F>,
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

        /// Converts the environment into a [MultiblockEvmInput] using the commitment method which
        /// was specified in the builder during creation.
        ///
        /// This method uses [SteelVerifier] internally to link the individual [EvmEnv].
        pub async fn into_input(self) -> anyhow::Result<MultiblockEvmInput<F>> {
            ensure!(!self.env.0.is_empty(), "environment must not be empty");

            let mut inputs = Vec::with_capacity(self.env.0.len());

            let mut iter = self.env.0.into_values().peekable();
            while let Some(env) = iter.next() {
                match iter.peek_mut() {
                    Some(next_env) => {
                        let dist = next_env.header.number() - env.header().number();
                        let input = if dist > verifier::HISTORY_LIMIT && !env.spec_id.has_eip2935()
                        {
                            bail!("EIP-2935 required since distance between blocks is too large");
                        }
                        // distance between blocks is close enough, so that we can verify directly
                        else if dist <= verifier::EIP2935_HISTORY_LIMIT {
                            let commit = env.commitment();
                            SteelVerifier::preflight(next_env).verify(&commit).await?;
                            env.into_input().await?
                        }
                        // use the EIP-2935 history commit to manage larger distances
                        else {
                            let target = next_env.header.number() - verifier::EIP2935_HISTORY_LIMIT;
                            let builder = self
                                .builder
                                .to_block(env.header().seal())
                                .commitment_block_number(target);
                            let env = builder.build().await?.merge(env)?;

                            let commit = env.commitment();
                            SteelVerifier::preflight(next_env).verify(&commit).await?;
                            env.into_input().await?
                        };
                        inputs.push(input);
                    }
                    None => {
                        let input = self.builder.build_input(env).await?;
                        inputs.push(input);
                    }
                }
            }

            Ok(MultiblockEvmInput(inputs))
        }

        /// Returns a mutable reference to the environment corresponding to the block number.
        pub fn get_mut(&mut self, num: u64) -> Option<&mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.get_mut(&num)
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

        /// Gets a mutable iterator over the environments in order by their block number.
        pub fn iter_mut(
            &mut self,
        ) -> impl Iterator<Item = &mut HostEvmEnv<ProviderDb<N, P>, F, ()>> {
            self.env.0.values_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ethereum::{EthMultiblockEvmEnv, ETH_MAINNET_CHAIN_SPEC},
        host::HostMultiblockEvmEnv,
        test_utils::get_el_url,
        Account,
    };
    use alloy::{
        network::TransactionBuilder,
        node_bindings::Anvil,
        providers::{Provider, ProviderBuilder},
    };
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

        let builder = EthMultiblockEvmEnv::builder()
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
            let info = Account::new(ADDRESS, guest_env.get(i as u64).unwrap()).info();
            assert_eq!(info.balance, U256::from(i));
        }

        let commitment = dbg!(guest_env.into_commitment());
        assert_eq!(commitment.digest, block_hash);
        assert_eq!(commitment.configID, chain_spec.digest());

        Ok(())
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn history_commitment() -> anyhow::Result<()> {
        const N: u64 = 3;
        // TODO: Make this an Anvil provider, once Anvil has EIP-2935 support
        let provider = ProviderBuilder::new().connect_http(get_el_url());

        let block_number = provider.get_block_number().await?;
        let block_hash = provider
            .get_block_by_number(block_number.into())
            .await?
            .unwrap()
            .hash();

        let builder = EthMultiblockEvmEnv::builder()
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
        assert_eq!(commitment.digest, block_hash);
        assert_eq!(commitment.configID, ETH_MAINNET_CHAIN_SPEC.digest());

        Ok(())
    }
}
