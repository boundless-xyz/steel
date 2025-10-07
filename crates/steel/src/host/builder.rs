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

use super::BlockId;
use crate::{
    beacon::BeaconCommit,
    config::ChainSpec,
    ethereum::{EthChainSpec, EthEvmFactory},
    history::{Eip2935HistoryCommit, HistoryCommit},
    host::{
        db::{ProofDb, ProviderConfig, ProviderDb},
        BlockNumberOrTag, EthHostEvmEnv, HostCommit, HostEvmEnv,
    },
    CommitmentVersion, EvmBlockHeader, EvmEnv, EvmFactory, EvmInput, EvmSpecId,
};
use alloy::{
    network::{primitives::HeaderResponse, BlockResponse, Ethereum, Network},
    providers::{Provider, ProviderBuilder, RootProvider},
};
use alloy_primitives::{BlockHash, BlockNumber, Sealable, Sealed, B256};
use anyhow::{anyhow, ensure, Context, Result};
use std::{fmt::Display, future::Future, marker::PhantomData};
use url::Url;

impl<F: EvmFactory> EvmEnv<(), F, ()> {
    /// Creates a builder for building an environment.
    ///
    /// Create an Ethereum environment bast on the latest block:
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use url::Url;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let url = Url::parse("https://ethereum-rpc.publicnode.com")?;
    /// let env = EthEvmEnv::builder().rpc(url).chain_spec(&ETH_MAINNET_CHAIN_SPEC).build().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> EvmEnvBuilder<(), F, (), ()> {
        EvmEnvBuilder::new()
    }
}

/// Builder for constructing an [EvmEnv] instance on the host.
///
/// The [EvmEnvBuilder] is used to configure and create an [EvmEnv], which is the environment in
/// which the Ethereum Virtual Machine (EVM) operates. This builder provides flexibility in setting
/// up the EVM environment by allowing configuration of RPC endpoints, block numbers, and other
/// parameters.
///
/// # Usage
/// The builder can be created using [EvmEnv::builder()]. Various configurations can be chained to
/// customize the environment before calling the `build` function to create the final [EvmEnv].
#[derive(Clone, Debug)]
pub struct EvmEnvBuilder<P, F, S, B> {
    provider: P,
    provider_config: ProviderConfig,
    block: BlockId,
    chain_spec: S,
    commitment_config: B,
    phantom: PhantomData<fn() -> F>,
}

impl<F: EvmFactory> EvmEnvBuilder<(), F, (), ()> {
    pub(crate) fn new() -> Self {
        EvmEnvBuilder {
            provider: (),
            provider_config: ProviderConfig::default(),
            block: BlockId::default(),
            chain_spec: (),
            commitment_config: (),
            phantom: PhantomData,
        }
    }
}

impl<S> EvmEnvBuilder<(), EthEvmFactory, S, ()> {
    /// Sets the Ethereum HTTP RPC endpoint that will be used by the [EvmEnv].
    pub fn rpc(self, url: Url) -> EvmEnvBuilder<RootProvider<Ethereum>, EthEvmFactory, S, ()> {
        self.provider(ProviderBuilder::default().connect_http(url))
    }
}

impl<F: EvmFactory, S> EvmEnvBuilder<(), F, S, ()> {
    /// Sets a custom [Provider] that will be used by the [EvmEnv].
    pub fn provider<N, P>(self, provider: P) -> EvmEnvBuilder<P, F, S, ()>
    where
        N: Network,
        P: Provider<N>,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
    {
        EvmEnvBuilder {
            provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec: self.chain_spec,
            commitment_config: self.commitment_config,
            phantom: self.phantom,
        }
    }
}

impl<P, F: EvmFactory, B> EvmEnvBuilder<P, F, (), B> {
    /// Sets the [ChainSpec] that will be used by the [EvmEnv].
    pub fn chain_spec(
        self,
        chain_spec: &ChainSpec<F::SpecId>,
    ) -> EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, B> {
        EvmEnvBuilder {
            provider: self.provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec,
            commitment_config: self.commitment_config,
            phantom: self.phantom,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Eip2935History {
    commitment_block: BlockId,
}

impl<P, F, S> EvmEnvBuilder<P, F, S, ()> {
    /// Sets the block hash for the commitment block, which can be different from the execution
    /// block.
    ///
    /// This allows for historical state execution while maintaining security through a more recent
    /// commitment. The commitment block must be more recent than the execution block.
    ///
    /// Note that this feature requires the Prague EVM version or later, as it relies on
    /// [EIP-2935](https://eips.ethereum.org/EIPS/eip-2935).
    ///
    /// # Example
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use alloy_primitives::B256;
    /// # use url::Url;
    /// # use std::str::FromStr;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let commitment_hash = B256::from_str("0x1234...")?;
    /// let builder = EthEvmEnv::builder()
    ///     .rpc(Url::parse("https://ethereum-rpc.publicnode.com")?)
    ///     .block_number(1_000_000) // execute against historical state
    ///     .commitment_block_hash(commitment_hash) // commit to recent block
    ///     .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
    /// let env = builder.build().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn commitment_block_hash(self, hash: BlockHash) -> EvmEnvBuilder<P, F, S, Eip2935History> {
        self.commitment_block(BlockId::Hash(hash))
    }

    /// Sets the block number or block tag ("latest", "earliest", "pending")  for the commitment.
    ///
    /// See [EvmEnvBuilder::commitment_block_hash] for detailed documentation.
    pub fn commitment_block_number_or_tag(
        self,
        block: BlockNumberOrTag,
    ) -> EvmEnvBuilder<P, F, S, Eip2935History> {
        self.commitment_block(BlockId::Number(block))
    }

    /// Sets the block number for the commitment.
    ///
    /// See [EvmEnvBuilder::commitment_block_hash] for detailed documentation.
    pub fn commitment_block_number(
        self,
        number: BlockNumber,
    ) -> EvmEnvBuilder<P, F, S, Eip2935History> {
        self.commitment_block_number_or_tag(BlockNumberOrTag::Number(number))
    }

    fn commitment_block(self, block: BlockId) -> EvmEnvBuilder<P, F, S, Eip2935History> {
        EvmEnvBuilder {
            provider: self.provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec: self.chain_spec,
            commitment_config: Eip2935History {
                commitment_block: block,
            },
            phantom: Default::default(),
        }
    }
}

/// Config for commitments to the beacon chain state.
#[derive(Clone, Debug)]
pub struct Beacon {
    url: Url,
    commitment_version: CommitmentVersion,
}

impl<P, S> EvmEnvBuilder<P, EthEvmFactory, S, ()> {
    /// Sets the Beacon API URL for retrieving Ethereum Beacon block root commitments.
    ///
    /// This function configures the [EvmEnv] to interact with an Ethereum Beacon chain.
    /// It assumes the use of the [mainnet](https://github.com/ethereum/consensus-specs/blob/v1.4.0/configs/mainnet.yaml) preset for consensus specs.
    pub fn beacon_api(self, url: Url) -> EvmEnvBuilder<P, EthEvmFactory, S, Beacon> {
        EvmEnvBuilder {
            provider: self.provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec: self.chain_spec,
            commitment_config: Beacon {
                url,
                commitment_version: CommitmentVersion::Beacon,
            },
            phantom: self.phantom,
        }
    }
}

impl<P, F, S, B> EvmEnvBuilder<P, F, S, B> {
    /// Sets the block number to be used for the EVM execution.
    pub fn block_number(self, number: u64) -> Self {
        self.block_number_or_tag(BlockNumberOrTag::Number(number))
    }

    /// Sets the block number or block tag ("latest", "earliest", "pending") to be used for the EVM
    /// execution.
    pub fn block_number_or_tag(mut self, block: BlockNumberOrTag) -> Self {
        self.block = BlockId::Number(block);
        self
    }

    /// Sets the block hash to be used for the EVM execution.
    pub fn block_hash(mut self, hash: B256) -> Self {
        self.block = BlockId::Hash(hash);
        self
    }

    /// Sets the chunk size for `eth_getProof` calls (EIP-1186).
    ///
    /// This configures the number of storage keys to request in a single call.
    /// The default is 1000, but this can be adjusted based on the RPC node configuration.
    pub fn eip1186_proof_chunk_size(mut self, chunk_size: usize) -> Self {
        assert_ne!(chunk_size, 0, "chunk size must be non-zero");
        self.provider_config.eip1186_proof_chunk_size = chunk_size;
        self
    }

    /// Returns a copy of the builder with elided commitment config and set EVM execution block.
    pub(crate) fn to_block(&self, block: impl Into<BlockId>) -> EvmEnvBuilder<P, F, S, ()>
    where
        P: Clone,
        S: Clone,
    {
        EvmEnvBuilder {
            provider: self.provider.clone(),
            provider_config: self.provider_config.clone(),
            block: block.into(),
            chain_spec: self.chain_spec.clone(),
            commitment_config: (),
            phantom: PhantomData,
        }
    }

    /// Returns the [EvmBlockHeader] of the specified block.
    ///
    /// If `block` is `None`, the block based on the current builder configuration is used instead.
    async fn get_header<N>(&self, block: Option<BlockId>) -> Result<Sealed<F::Header>>
    where
        F: EvmFactory,
        N: Network,
        P: Provider<N>,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
    {
        let block = block.unwrap_or(self.block);
        let block = block.into_rpc_type(&self.provider).await?;

        let rpc_block = self
            .provider
            .get_block(block)
            .await
            .context("eth_getBlock failed")?
            .with_context(|| format!("block {block} not found"))?;

        let rpc_header = rpc_block.header().clone();
        let header: F::Header = rpc_header
            .try_into()
            .map_err(|err| anyhow!("header invalid: {err}"))?;
        let header = header.seal_slow();
        ensure!(
            header.seal() == rpc_block.header().hash(),
            "computed block hash does not match the hash returned by the API"
        );

        Ok(header)
    }
}

impl<P, F: EvmFactory> EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, ()> {
    /// Builds and returns an [EvmEnv] with the configured settings that commits to a block hash.
    pub async fn build<N>(self) -> Result<HostEvmEnv<ProviderDb<N, P>, F, ()>>
    where
        N: Network,
        P: Provider<N>,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
    {
        let header = self.get_header(None).await?;
        log::debug!(
            "Environment initialized with block {} ({})",
            header.number(),
            header.seal()
        );

        create_host_env::<N, P, F, _>(
            self.provider,
            self.provider_config,
            self.chain_spec,
            header,
            HostCommit {
                inner: (),
                config_id: self.chain_spec.digest(),
            },
        )
    }
}

impl<P, F: EvmFactory> EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, Eip2935History> {
    /// Builds and returns an [EvmEnv] with the configured settings that commits to a block hash.
    pub async fn build<N>(
        self,
    ) -> Result<HostEvmEnv<ProviderDb<N, P>, F, Eip2935HistoryCommit<F::Header>>>
    where
        N: Network,
        P: Provider<N>,
        F::Header: TryFrom<<N as Network>::HeaderResponse>,
        <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
    {
        let evm_header = self.get_header(None).await?;
        let commitment_header = self
            .get_header(Some(self.commitment_config.commitment_block))
            .await?;

        log::debug!(
            "Environment initialized with block {} ({})",
            evm_header.number(),
            evm_header.seal()
        );

        let history_commit =
            Eip2935HistoryCommit::from_headers(&evm_header, &commitment_header, &self.provider)
                .await?;
        let commit = HostCommit {
            inner: history_commit,
            config_id: self.chain_spec.digest(),
        };
        let env = create_host_env::<N, P, F, _>(
            self.provider,
            self.provider_config,
            self.chain_spec,
            evm_header,
            commit,
        )?;
        ensure!(env.spec_id().has_eip2935(), "EIP-2935 not supported");

        Ok(env)
    }
}

/// Config for separating the execution block from the commitment block.
#[derive(Clone, Debug)]
pub struct History {
    beacon_config: Beacon,
    commitment_block: BlockId,
}

impl<P, S> EvmEnvBuilder<P, EthEvmFactory, S, Beacon> {
    /// Configures the environment builder to generate consensus commitments.
    ///
    /// A consensus commitment contains the beacon block root indexed directly by its slot number.
    /// This is in contrast to the default mechanism, which relies on timestamps for lookups, for
    /// verification using the EIP-4788 beacon root contract deployed at the execution layer.
    ///
    /// The use of slot-based indexing is particularly beneficial for verification methods that have
    /// direct access to the state of the beacon chain, such as systems using beacon light clients.
    /// This allows the commitment to be verified directly against the state of the consensus layer.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use alloy_primitives::B256;
    /// # use url::Url;
    /// # use std::str::FromStr;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let builder = EthEvmEnv::builder()
    ///     .rpc(Url::parse("https://ethereum-rpc.publicnode.com")?)
    ///     .beacon_api(Url::parse("https://ethereum-beacon-api.publicnode.com")?)
    ///     .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
    ///     // Configure the builder to use slot-indexed consensus commitments.
    ///     .consensus_commitment();
    ///
    /// // The resulting 'env' will be configured to generate a consensus commitment
    /// // (beacon root indexed by slot) when processing blocks or state.
    /// let env = builder.build().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn consensus_commitment(mut self) -> Self {
        self.commitment_config.commitment_version = CommitmentVersion::Consensus;
        self
    }

    /// Sets the block hash for the commitment block, which can be different from the execution
    /// block.
    ///
    /// This allows for historical state execution while maintaining security through a more recent
    /// commitment. The commitment block must be more recent than the execution block.
    ///
    /// Note that this feature requires a Beacon chain RPC provider, as it relies on
    /// [EIP-4788](https://eips.ethereum.org/EIPS/eip-4788).
    ///
    /// # Example
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use alloy_primitives::B256;
    /// # use url::Url;
    /// # use std::str::FromStr;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let commitment_hash = B256::from_str("0x1234...")?;
    /// let builder = EthEvmEnv::builder()
    ///     .rpc(Url::parse("https://ethereum-rpc.publicnode.com")?)
    ///     .beacon_api(Url::parse("https://ethereum-beacon-api.publicnode.com")?)
    ///     .block_number(1_000_000) // execute against historical state
    ///     .commitment_block_hash(commitment_hash) // commit to recent block
    ///     .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
    /// let env = builder.build().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn commitment_block_hash(
        self,
        hash: BlockHash,
    ) -> EvmEnvBuilder<P, EthEvmFactory, S, History> {
        self.commitment_block(BlockId::Hash(hash))
    }

    /// Sets the block number or block tag ("latest", "earliest", "pending")  for the commitment.
    ///
    /// See [EvmEnvBuilder::commitment_block_hash] for detailed documentation.
    pub fn commitment_block_number_or_tag(
        self,
        block: BlockNumberOrTag,
    ) -> EvmEnvBuilder<P, EthEvmFactory, S, History> {
        self.commitment_block(BlockId::Number(block))
    }

    /// Sets the block number for the commitment.
    ///
    /// See [EvmEnvBuilder::commitment_block_hash] for detailed documentation.
    pub fn commitment_block_number(
        self,
        number: BlockNumber,
    ) -> EvmEnvBuilder<P, EthEvmFactory, S, History> {
        self.commitment_block_number_or_tag(BlockNumberOrTag::Number(number))
    }

    fn commitment_block(self, block: BlockId) -> EvmEnvBuilder<P, EthEvmFactory, S, History> {
        EvmEnvBuilder {
            provider: self.provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec: self.chain_spec,
            commitment_config: History {
                beacon_config: self.commitment_config,
                commitment_block: block,
            },
            phantom: Default::default(),
        }
    }
}

impl<P> EvmEnvBuilder<P, EthEvmFactory, &ChainSpec<<EthEvmFactory as EvmFactory>::SpecId>, Beacon> {
    /// Builds and returns an [EvmEnv] with the configured settings that commits to a beacon root.
    pub async fn build(self) -> Result<EthHostEvmEnv<ProviderDb<Ethereum, P>, BeaconCommit>>
    where
        P: Provider<Ethereum>,
    {
        let header = self.get_header(None).await?;
        log::debug!(
            "Environment initialized with block {} ({})",
            header.number(),
            header.seal()
        );

        let beacon_url = self.commitment_config.url;
        let version = self.commitment_config.commitment_version;
        let commit = HostCommit {
            inner: BeaconCommit::from_header(&header, version, &self.provider, beacon_url).await?,
            config_id: self.chain_spec.digest(),
        };

        create_host_env(
            self.provider,
            self.provider_config,
            self.chain_spec,
            header,
            commit,
        )
    }
}

impl<P>
    EvmEnvBuilder<P, EthEvmFactory, &ChainSpec<<EthEvmFactory as EvmFactory>::SpecId>, History>
{
    /// Configures the environment builder to generate consensus commitments.
    ///
    /// See [EvmEnvBuilder<P, EthEvmFactory, S, Beacon>::consensus_commitment] for more info.
    pub fn consensus_commitment(mut self) -> Self {
        self.commitment_config.beacon_config.commitment_version = CommitmentVersion::Consensus;
        self
    }

    /// Builds and returns an [EvmEnv] with the configured settings, using a dedicated commitment
    /// block that is different from the execution block.
    pub async fn build(self) -> Result<EthHostEvmEnv<ProviderDb<Ethereum, P>, HistoryCommit>>
    where
        P: Provider<Ethereum>,
    {
        let evm_header = self.get_header(None).await?;
        let commitment_header = self
            .get_header(Some(self.commitment_config.commitment_block))
            .await?;

        log::debug!(
            "Environment initialized with block {} ({})",
            evm_header.number(),
            evm_header.seal()
        );

        let beacon_url = self.commitment_config.beacon_config.url;
        let commitment_version = self.commitment_config.beacon_config.commitment_version;
        let history_commit = HistoryCommit::from_headers(
            &evm_header,
            &commitment_header,
            commitment_version,
            &self.provider,
            beacon_url,
        )
        .await?;
        let commit = HostCommit {
            inner: history_commit,
            config_id: self.chain_spec.digest(),
        };
        let env = create_host_env::<Ethereum, P, EthEvmFactory, _>(
            self.provider,
            self.provider_config,
            self.chain_spec,
            evm_header,
            commit,
        )?;
        ensure!(env.spec_id().has_eip4788(), "EIP-4788 not supported");

        Ok(env)
    }
}

fn create_host_env<N: Network, P: Provider<N>, F: EvmFactory, C>(
    provider: P,
    provider_config: ProviderConfig,
    chain_spec: &ChainSpec<F::SpecId>,
    header: Sealed<F::Header>,
    commit: HostCommit<C>,
) -> Result<HostEvmEnv<ProviderDb<N, P>, F, C>> {
    let db = ProofDb::new(ProviderDb::new(provider, provider_config, header.seal()));
    let chain_id = chain_spec.chain_id();
    let spec_id = *chain_spec.active_fork(header.number(), header.timestamp())?;

    Ok(EvmEnv::new(db, chain_id, spec_id, header, commit))
}

/// Extension trait used by [HostMultiblockEvmEnv] to build an [EvmInput] instance from an
/// [EvmEnvBuilder] given an existing [EvmEnv].
///
/// This trait abstracts the process of transforming a configured [EvmEnvBuilder] into a
/// corresponding [EvmInput]. Essentially, it applies the specified commitment type from the
/// [EvmEnvBuilder] to the EVM data from the given [EvmEnv].
///
/// [HostMultiblockEvmEnv]: crate::multiblock::host::HostMultiblockEvmEnv
pub trait InputBuilder<D, F: EvmFactory>: Send {
    /// Consumes this builder and constructs an [EvmInput] from the given [EvmEnv].
    ///
    /// The returned future performs any necessary commitment computation, or state verification
    /// required by the builder’s configuration.
    /// It returns an error, if this process fails or if the [ChainSpec] config of the
    /// [EvmEnvBuilder] and the [EvmEnv] do not match.
    fn build_input(
        self,
        env: HostEvmEnv<D, F, ()>,
    ) -> impl Future<Output = Result<EvmInput<F>>> + Send;
}

macro_rules! build_input {
    ($D:ty, $F:ty) => {
        async fn build_input(self, env: HostEvmEnv<$D, $F, ()>) -> Result<EvmInput<$F>> {
            // rebuild an empty environment for the same block
            let builder = self.block_hash(env.header().seal());
            let empty_env = builder.build().await.context("builder failed")?;
            // merge execution state and verify compatibility
            let env = empty_env
                .merge(env)
                .context("environment not compatible with builder")?;

            env.into_input().await
        }
    };
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
    build_input!(ProviderDb<N, P>, F);
}

impl<N, P, F: EvmFactory> InputBuilder<ProviderDb<N, P>, F>
    for EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, Eip2935History>
where
    N: Network,
    P: Provider<N>,
    F::Header: TryFrom<<N as Network>::HeaderResponse>,
    <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
    F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
    <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
{
    build_input!(ProviderDb<N, P>, F);
}

impl<P: Provider<Ethereum>> InputBuilder<ProviderDb<Ethereum, P>, EthEvmFactory>
    for EvmEnvBuilder<P, EthEvmFactory, &EthChainSpec, Beacon>
{
    build_input!(ProviderDb<Ethereum, P>, EthEvmFactory);
}

impl<P: Provider<Ethereum>> InputBuilder<ProviderDb<Ethereum, P>, EthEvmFactory>
    for EvmEnvBuilder<P, EthEvmFactory, &EthChainSpec, History>
{
    build_input!(ProviderDb<Ethereum, P>, EthEvmFactory);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ethereum::{EthEvmEnv, ETH_MAINNET_CHAIN_SPEC},
        test_utils::{get_cl_url, get_el_url},
        BlockHeaderCommit, Commitment, CommitmentVersion,
    };
    use alloy_consensus::BlockHeader;
    use test_log::test;

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_block_env() {
        let builder = EthEvmEnv::builder()
            .rpc(get_el_url())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        // the builder should be cloneable
        builder.clone().build().await.unwrap();
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_beacon_env() {
        let provider = ProviderBuilder::default().connect_http(get_el_url());

        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .beacon_api(get_cl_url())
            .block_number_or_tag(BlockNumberOrTag::Parent)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();
        let commit = env.commit.inner.commit(&env.header, env.commit.config_id);

        // the commitment should verify against the parent_beacon_block_root of the child
        let child_block = provider
            .get_block_by_number((env.header.number() + 1).into())
            .await
            .unwrap();
        let header = child_block.unwrap().header;
        assert_eq!(
            commit,
            Commitment::new(
                CommitmentVersion::Beacon as u16,
                header.timestamp,
                header.parent_beacon_block_root.unwrap(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_history_env() {
        let provider = ProviderBuilder::default().connect_http(get_el_url());

        // initialize the env at latest - 100 while committing to latest - 1
        let latest = provider.get_block_number().await.unwrap();
        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .block_number_or_tag(BlockNumberOrTag::Number(latest - 10_000))
            .beacon_api(get_cl_url())
            .commitment_block_number(latest - 1)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();
        let commit = env.commit.inner.commit(&env.header, env.commit.config_id);

        // the commitment should verify against the parent_beacon_block_root of the latest block
        let child_block = provider.get_block_by_number(latest.into()).await.unwrap();
        let header = child_block.unwrap().header;
        assert_eq!(
            commit,
            Commitment::new(
                CommitmentVersion::Beacon as u16,
                header.timestamp,
                header.parent_beacon_block_root.unwrap(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_2935_history_env() {
        let provider = ProviderBuilder::new().connect_http(get_el_url());

        let latest_header = provider
            .get_block_by_number(alloy::eips::eip1898::BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap()
            .header;

        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .block_number_or_tag(BlockNumberOrTag::Number(latest_header.number() - 10_000))
            .commitment_block_hash(latest_header.hash())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();
        let commit = env.commit.inner.commit(&env.header, env.commit.config_id);

        assert_eq!(
            commit,
            Commitment::new(
                CommitmentVersion::Block as u16,
                latest_header.number(),
                latest_header.hash(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }
}
