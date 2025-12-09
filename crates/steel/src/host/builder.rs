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
    CommitmentVersion, EvmBlockHeader, EvmEnv, EvmFactory, EvmSpecId,
    beacon::{BeaconCommit, host::client::BeaconClient},
    config::ChainSpec,
    ethereum::EthEvmFactory,
    history::{Eip2935HistoryCommit, HistoryCommit},
    host::{
        BlockId, BlockNumberOrTag, EthHostEvmEnv, HostCommit, HostEvmEnv,
        db::{ProofDb, ProviderConfig, ProviderDb},
    },
};
use alloy::{
    network::{BlockResponse, Ethereum, Network, primitives::HeaderResponse},
    providers::{Provider, ProviderBuilder, RootProvider},
};
use alloy_primitives::{B256, BlockHash, BlockNumber, Sealable, Sealed};
use anyhow::{Context, Result, anyhow, ensure};
use std::{fmt::Display, marker::PhantomData};
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
pub struct EvmEnvBuilder<P, F, S, C> {
    provider: P,
    provider_config: ProviderConfig,
    block: BlockId,
    chain_spec: S,
    commitment_config: C,
    phantom: PhantomData<F>,
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

impl<P, F: EvmFactory, C> EvmEnvBuilder<P, F, (), C> {
    /// Sets the [ChainSpec] that will be used by the [EvmEnv].
    pub fn chain_spec(
        self,
        chain_spec: &ChainSpec<F::SpecId>,
    ) -> EvmEnvBuilder<P, F, &ChainSpec<F::SpecId>, C> {
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

/// Config for [Eip2935HistoryCommit] creation.
#[derive(Clone, Debug)]
pub struct Eip2935History {
    target: BlockId,
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
            commitment_config: Eip2935History { target: block },
            phantom: Default::default(),
        }
    }
}

/// Config for [BeaconCommit] creation.
#[derive(Clone, Debug)]
pub struct Beacon {
    url: Url,
    version: CommitmentVersion,
}

impl Beacon {
    fn client(&self) -> Result<BeaconClient> {
        BeaconClient::new(self.url.clone()).context("invalid Ethereum Beacon API URL")
    }
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
                version: CommitmentVersion::Beacon,
            },
            phantom: self.phantom,
        }
    }
}

impl<P, F, S, C> EvmEnvBuilder<P, F, S, C> {
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
        .await
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
        let commitment_header = self.get_header(Some(self.commitment_config.target)).await?;

        // If the blocks are the same, the overhead of Eip2935HistoryCommit is unnecessary,
        // and the logic in `from_headers` (which expects strict inequality) will fail.
        ensure_distinct(&evm_header, &commitment_header)?;

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
        )
        .await?;
        ensure!(env.spec_id().has_eip2935(), "EIP-2935 not supported");

        Ok(env)
    }
}

/// Config for [HistoryCommit] creation.
#[derive(Clone, Debug)]
pub struct History {
    config: Beacon,
    target: CommitmentTarget,
}

/// The target of a history commitment can either be an execution block or a beacon slot.
#[derive(Clone, Debug)]
enum CommitmentTarget {
    Block(BlockId),
    Slot(u64),
}

impl<P, S> EvmEnvBuilder<P, EthEvmFactory, S, Beacon> {
    /// Configures the environment builder to generate consensus commitments.
    ///
    /// A consensus commitment contains the beacon block root indexed by its slot number, rather
    /// than by timestamp. The default beacon commitment uses timestamp-based lookups, which can be
    /// verified on-chain using the EIP-4788 beacon root contract. Consensus commitments instead
    /// allow direct verification against the beacon chain state, making them ideal for systems
    /// using beacon light clients.
    ///
    /// For historical state execution with consensus commitments, see
    /// [EvmEnvBuilder::consensus_commitment_slot()], which allows specifying a
    /// more recent beacon slot as the commitment target.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use url::Url;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let env = EthEvmEnv::builder()
    ///     .rpc(Url::parse("https://ethereum-rpc.publicnode.com")?)
    ///     .beacon_api(Url::parse("https://ethereum-beacon-api.publicnode.com")?)
    ///     .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
    ///     .consensus_commitment()
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn consensus_commitment(mut self) -> Self {
        self.commitment_config.version = CommitmentVersion::Consensus;
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
                config: self.commitment_config,
                target: CommitmentTarget::Block(block),
            },
            phantom: Default::default(),
        }
    }

    /// Sets the Beacon Chain slot for the commitment.
    ///
    /// This allows specifying an exact slot number to commit to, rather than deriving it from an
    /// execution block. This is particularly useful for light client verification scenarios, where
    /// the verifier has direct access to the beacon chain state and can look up beacon block roots
    /// by slot number.
    ///
    /// Note that this creates a historical commitment, meaning the execution block and commitment
    /// block will be different. The commitment slot must correspond to a block more recent than the
    /// execution block.
    ///
    /// # Example
    /// ```rust,no_run
    /// # use risc0_steel::ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv};
    /// # use url::Url;
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> anyhow::Result<()> {
    /// let builder = EthEvmEnv::builder()
    ///     .rpc(Url::parse("https://ethereum-rpc.publicnode.com")?)
    ///     .beacon_api(Url::parse("https://ethereum-beacon-api.publicnode.com")?)
    ///     .block_number(19_000_000) // execute against historical state
    ///     .consensus_commitment_slot(9_500_000) // commit to a specific beacon slot
    ///     .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
    /// let env = builder.build().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [(EvmEnvBuilder::consensus_commitment()] for more info on consensus commitments.
    pub fn consensus_commitment_slot(
        self,
        slot: u64,
    ) -> EvmEnvBuilder<P, EthEvmFactory, S, History> {
        EvmEnvBuilder {
            provider: self.provider,
            provider_config: self.provider_config,
            block: self.block,
            chain_spec: self.chain_spec,
            commitment_config: History {
                config: Beacon {
                    url: self.commitment_config.url,
                    version: CommitmentVersion::Consensus,
                },
                target: CommitmentTarget::Slot(slot),
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

        let client = self.commitment_config.client()?;
        let version = self.commitment_config.version;

        let commit = HostCommit {
            inner: BeaconCommit::from_header(&header, version, &self.provider, &client).await?,
            config_id: self.chain_spec.digest(),
        };

        create_host_env(
            self.provider,
            self.provider_config,
            self.chain_spec,
            header,
            commit,
        )
        .await
    }
}

impl<P>
    EvmEnvBuilder<P, EthEvmFactory, &ChainSpec<<EthEvmFactory as EvmFactory>::SpecId>, History>
{
    /// Builds and returns an [EvmEnv] with the configured settings, using a dedicated commitment
    /// block that is different from the execution block.
    pub async fn build(self) -> Result<EthHostEvmEnv<ProviderDb<Ethereum, P>, HistoryCommit>>
    where
        P: Provider<Ethereum>,
    {
        let evm_header = self.get_header(None).await?;

        let beacon_client = self.commitment_config.config.client()?;
        let commitment_version = self.commitment_config.config.version;
        let commitment_header = match self.commitment_config.target {
            CommitmentTarget::Block(block) => self.get_header(Some(block)).await?,
            CommitmentTarget::Slot(slot) => {
                // Sanity check: This invariant should be guaranteed by the builder methods
                assert_eq!(
                    commitment_version,
                    CommitmentVersion::Consensus,
                    "commitment slot target requires consensus commitment version"
                );

                // resolve slot -> execution header
                let block_hash = beacon_client
                    .get_execution_payload_block_hash(slot)
                    .await
                    .with_context(|| format!("failed to get block hash for beacon slot {slot}"))?;
                self.get_header(Some(block_hash.into())).await?
            }
        };

        // If the blocks are the same, the overhead of HistoryCommit is unnecessary,
        // and the logic in `from_headers` (which expects strict inequality) will fail.
        ensure_distinct(&evm_header, &commitment_header)?;

        let history_commit = HistoryCommit::from_headers(
            &evm_header,
            &commitment_header,
            commitment_version,
            &self.provider,
            &beacon_client,
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
        )
        .await?;
        ensure!(env.spec_id().has_eip4788(), "EIP-4788 not supported");

        Ok(env)
    }
}

async fn create_host_env<N: Network, P: Provider<N>, F: EvmFactory, C>(
    provider: P,
    provider_config: ProviderConfig,
    chain_spec: &ChainSpec<F::SpecId>,
    header: Sealed<F::Header>,
    commit: HostCommit<C>,
) -> Result<HostEvmEnv<ProviderDb<N, P>, F, C>> {
    // perform a sanity check to ensure that the provider matches the specifications
    let provider_chain_id = provider
        .get_chain_id()
        .await
        .context("eth_chainId failed")?;
    if provider_chain_id != chain_spec.chain_id {
        log::warn!(
            "Chain ID mismatch: provider returned {provider_chain_id}, but chain spec expects {}",
            chain_spec.chain_id
        );
    }

    log::debug!(
        "Environment initialized with block {} ({})",
        header.number(),
        header.seal()
    );

    let db = ProofDb::new(ProviderDb::new(provider, provider_config, header.seal()));
    let chain_id = chain_spec.chain_id();
    let spec_id = *chain_spec.active_fork(header.number(), header.timestamp())?;

    Ok(EvmEnv::new(db, chain_id, spec_id, header, commit))
}

fn ensure_distinct<H: EvmBlockHeader>(
    evm_header: &Sealed<H>,
    commitment_header: &Sealed<H>,
) -> Result<()> {
    ensure!(
        evm_header.seal() != commitment_header.seal(),
        "The execution block ({}) matches the commitment block. \
        Historical proofs are unnecessary in this case. \
        Remove the explicit commitment target to use a direct commitment.",
        evm_header.number()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Commitment, CommitmentVersion,
        ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv},
        test_utils::{get_cl_url, get_el_url},
    };
    use alloy_consensus::BlockHeader;
    use test_log::test;

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_env_with_block_commit() {
        let builder = EthEvmEnv::builder()
            .rpc(get_el_url())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        // the builder should be cloneable
        let env = builder.clone().build().await.unwrap();

        assert_eq!(
            env.commitment(),
            Commitment::new(
                CommitmentVersion::Block as u16,
                env.header.number(),
                env.header.seal(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_env_with_beacon_commit() {
        let provider = ProviderBuilder::default().connect_http(get_el_url());

        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .beacon_api(get_cl_url())
            .block_number_or_tag(BlockNumberOrTag::Parent)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();

        // the commitment should verify against the parent_beacon_block_root of the child
        let child_block = provider
            .get_block_by_number((env.header.number() + 1).into())
            .await
            .unwrap();
        let header = child_block.unwrap().header;
        assert_eq!(
            env.commitment(),
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
    async fn build_env_with_consensus_commit() {
        let cl_url = get_cl_url();
        let beacon_client = BeaconClient::new(cl_url.clone()).unwrap();
        let beacon_head = beacon_client.get_block("head").await.unwrap();

        let block_hash = B256::from_slice(beacon_head.execution_payload().unwrap().block_hash());
        let builder = EthEvmEnv::builder()
            .rpc(get_el_url())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
            .block_hash(block_hash)
            .beacon_api(cl_url)
            .consensus_commitment();
        let env = builder.clone().build().await.unwrap();

        // the commitment should verify against the head beacon block
        assert_eq!(
            env.commitment(),
            Commitment::new(
                CommitmentVersion::Consensus as u16,
                beacon_head.slot(),
                beacon_head.root().unwrap(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_env_with_history_beacon_commit() {
        let provider = ProviderBuilder::default().connect_http(get_el_url());

        // initialize the env at latest - 10,000 while committing to latest - 1
        let latest = provider.get_block_number().await.unwrap();
        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .block_number_or_tag(BlockNumberOrTag::Number(latest - 10_000))
            .beacon_api(get_cl_url())
            .commitment_block_number(latest - 1)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();

        // the commitment should verify against the parent_beacon_block_root of the latest block
        let child_block = provider.get_block_by_number(latest.into()).await.unwrap();
        let header = child_block.unwrap().header;
        assert_eq!(
            env.commitment(),
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
    async fn build_env_with_history_consensus_commit() {
        let provider = ProviderBuilder::default().connect_http(get_el_url());
        let cl_url = get_cl_url();
        let beacon_client = BeaconClient::new(cl_url.clone()).unwrap();
        let beacon_head = beacon_client.get_block("head").await.unwrap();

        // initialize the env at latest - 10,000 while committing to the head of the beacon chain
        let latest = provider.get_block_number().await.unwrap();
        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .block_number_or_tag(BlockNumberOrTag::Number(latest - 10_000))
            .beacon_api(cl_url)
            .consensus_commitment_slot(beacon_head.slot())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();

        // the commitment should verify against the head beacon block
        assert_eq!(
            env.commitment(),
            Commitment::new(
                CommitmentVersion::Consensus as u16,
                beacon_head.slot(),
                beacon_head.root().unwrap(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn build_env_with_2935_history_block_commit() {
        let provider = ProviderBuilder::new().connect_http(get_el_url());
        let latest_header = get_latest_header(&provider).await.unwrap();

        let builder = EthEvmEnv::builder()
            .provider(&provider)
            .block_number_or_tag(BlockNumberOrTag::Number(latest_header.number() - 10_000))
            .commitment_block_hash(latest_header.hash())
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC);
        let env = builder.clone().build().await.unwrap();

        assert_eq!(
            env.commitment(),
            Commitment::new(
                CommitmentVersion::Block as u16,
                latest_header.number(),
                latest_header.hash(),
                ETH_MAINNET_CHAIN_SPEC.digest(),
            )
        );
    }

    async fn get_latest_header(provider: impl Provider) -> Result<alloy::rpc::types::Header> {
        Ok(provider
            .get_block_by_number(alloy::eips::BlockNumberOrTag::Latest)
            .await?
            .context("no latest header")?
            .header)
    }
}
