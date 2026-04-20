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

use crate::{
    DisputeGameCommit,
    game::host::{DisputeGameIndex, OptimismPortal2},
    optimism::{OpBlockHeader, OpChainSpec, OpEvmFactory, OpEvmInput},
};
use alloy::{
    network::{Ethereum, Network, TransactionBuilder},
    providers::{
        Provider, ProviderBuilder, RootProvider,
        fillers::{ChainIdFiller, GasFiller, JoinFill, NonceFiller, RecommendedFillers},
    },
    rpc::types::{AccessList, eth as rpc_types},
};
use alloy_consensus::{Header, ReceiptWithBloom, TxType};
use alloy_primitives::{Address, Bytes, ChainId, Sealable, TxKind, U256};
use anyhow::{Context, Result};
use op_alloy_consensus::{OpReceipt, OpTxEnvelope, OpTxType, OpTypedTransaction};
use op_alloy_rpc_types::{OpTransactionReceipt, OpTransactionRequest};
use risc0_steel::{
    BlockHeaderCommit, Commitment, ComposeInput, EvmEnv, EvmInput,
    host::{
        BlockNumberOrTag, EvmEnvBuilder, HostCommit,
        db::{ProofDb, ProviderDb},
    },
};
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};
use url::Url;

/// TODO(https://github.com/boundless-xyz/steel/issues/116): replace with `op_alloy_network::Optimism`
/// once a new `op-alloy-network` version is released with the `NetworkWallet` conflict fix.
///
/// Types for an OP-stack network.
#[derive(Clone, Copy, Debug)]
pub struct Optimism(());

impl Network for Optimism {
    type TxType = OpTxType;
    type TxEnvelope = OpTxEnvelope;
    type UnsignedTx = OpTypedTransaction;
    type ReceiptEnvelope = ReceiptWithBloom<OpReceipt>;
    type Header = Header;
    type TransactionRequest = OpTransactionRequest;
    type TransactionResponse = op_alloy_rpc_types::Transaction;
    type ReceiptResponse = OpTransactionReceipt;
    type HeaderResponse = rpc_types::Header;
    type BlockResponse = rpc_types::Block<Self::TransactionResponse, Self::HeaderResponse>;
}

impl TransactionBuilder<Optimism> for OpTransactionRequest {
    fn chain_id(&self) -> Option<ChainId> {
        self.as_ref().chain_id()
    }
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.as_mut().set_chain_id(chain_id);
    }
    fn nonce(&self) -> Option<u64> {
        self.as_ref().nonce()
    }
    fn set_nonce(&mut self, nonce: u64) {
        self.as_mut().set_nonce(nonce);
    }
    fn take_nonce(&mut self) -> Option<u64> {
        self.as_mut().nonce.take()
    }
    fn input(&self) -> Option<&Bytes> {
        self.as_ref().input()
    }
    fn set_input<T: Into<Bytes>>(&mut self, input: T) {
        self.as_mut().set_input(input);
    }
    fn from(&self) -> Option<Address> {
        self.as_ref().from()
    }
    fn set_from(&mut self, from: Address) {
        self.as_mut().set_from(from);
    }
    fn kind(&self) -> Option<TxKind> {
        self.as_ref().kind()
    }
    fn clear_kind(&mut self) {
        self.as_mut().clear_kind();
    }
    fn set_kind(&mut self, kind: TxKind) {
        self.as_mut().set_kind(kind);
    }
    fn value(&self) -> Option<U256> {
        self.as_ref().value()
    }
    fn set_value(&mut self, value: U256) {
        self.as_mut().set_value(value);
    }
    fn gas_price(&self) -> Option<u128> {
        self.as_ref().gas_price()
    }
    fn set_gas_price(&mut self, gas_price: u128) {
        self.as_mut().set_gas_price(gas_price);
    }
    fn max_fee_per_gas(&self) -> Option<u128> {
        self.as_ref().max_fee_per_gas()
    }
    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        self.as_mut().set_max_fee_per_gas(max_fee_per_gas);
    }
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.as_ref().max_priority_fee_per_gas()
    }
    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        self.as_mut()
            .set_max_priority_fee_per_gas(max_priority_fee_per_gas);
    }
    fn gas_limit(&self) -> Option<u64> {
        self.as_ref().gas_limit()
    }
    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.as_mut().set_gas_limit(gas_limit);
    }
    fn access_list(&self) -> Option<&AccessList> {
        self.as_ref().access_list()
    }
    fn set_access_list(&mut self, access_list: AccessList) {
        self.as_mut().set_access_list(access_list);
    }
    fn complete_type(&self, ty: OpTxType) -> Result<(), Vec<&'static str>> {
        match ty {
            OpTxType::Deposit => Err(vec!["not implemented for deposit tx"]),
            _ => {
                let ty = TxType::try_from(ty as u8).unwrap();
                self.as_ref().complete_type(ty)
            }
        }
    }
    fn can_submit(&self) -> bool {
        self.as_ref().can_submit()
    }
    fn can_build(&self) -> bool {
        self.as_ref().can_build()
    }
    fn output_tx_type(&self) -> OpTxType {
        match self.as_ref().preferred_type() {
            TxType::Eip1559 | TxType::Eip4844 => OpTxType::Eip1559,
            TxType::Eip2930 => OpTxType::Eip2930,
            TxType::Eip7702 => OpTxType::Eip7702,
            TxType::Legacy => OpTxType::Legacy,
        }
    }
    fn output_tx_type_checked(&self) -> Option<OpTxType> {
        self.as_ref().buildable_type().map(|tx_ty| match tx_ty {
            TxType::Eip1559 | TxType::Eip4844 => OpTxType::Eip1559,
            TxType::Eip2930 => OpTxType::Eip2930,
            TxType::Eip7702 => OpTxType::Eip7702,
            TxType::Legacy => OpTxType::Legacy,
        })
    }
    fn prep_for_submission(&mut self) {
        self.as_mut().prep_for_submission();
    }
    fn build_unsigned(self) -> alloy::network::BuildResult<OpTypedTransaction, Optimism> {
        if let Err((tx_type, missing)) = self.as_ref().missing_keys() {
            let tx_type = OpTxType::try_from(tx_type as u8).unwrap();
            return Err(
                alloy::network::TransactionBuilderError::InvalidTransactionRequest(
                    tx_type, missing,
                )
                .into_unbuilt(self),
            );
        }
        Ok(self.build_typed_tx().expect("checked by missing_keys"))
    }
    async fn build<W: alloy::network::NetworkWallet<Optimism>>(
        self,
        wallet: &W,
    ) -> Result<<Optimism as Network>::TxEnvelope, alloy::network::TransactionBuilderError<Optimism>>
    {
        Ok(wallet.sign_request(self).await?)
    }
}

impl RecommendedFillers for Optimism {
    type RecommendedFillers = JoinFill<GasFiller, JoinFill<NonceFiller, ChainIdFiller>>;

    fn recommended_fillers() -> Self::RecommendedFillers {
        Default::default()
    }
}

/// Wrapped [EvmEnv] for Optimism.
pub struct OpEvmEnv<D, C> {
    /// Underlying generic environment without a specific commitment.
    inner: EvmEnv<D, OpEvmFactory, HostCommit<()>>,
    /// Additional OP-specific commitment.
    commit: C,
}

impl<D, C> Deref for OpEvmEnv<D, C> {
    type Target = EvmEnv<D, OpEvmFactory, HostCommit<()>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<D, C> DerefMut for OpEvmEnv<D, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl OpEvmEnv<(), ()> {
    /// Initialize an OP-specific builder.
    pub fn builder() -> OpEvmEnvBuilder<PreProviderStage, (), (), ()> {
        OpEvmEnvBuilder {
            inner: EvmEnv::builder(),
            l2_provider: (),
            dispute_game_config: (),
            stage: PhantomData,
        }
    }
}

type HostOpEvmEnv<P2, C> = OpEvmEnv<ProofDb<ProviderDb<Optimism, P2>>, C>;

impl<P2> HostOpEvmEnv<P2, ()>
where
    P2: Provider<Optimism>,
{
    pub async fn into_input(self) -> Result<OpEvmInput> {
        // the inner environment has no specific commitment, so it will always return a block input
        let EvmInput::Block(input) = self.inner.into_input().await? else {
            unreachable!()
        };

        Ok(OpEvmInput::Block(input))
    }
}

impl<P2, C> HostOpEvmEnv<P2, C>
where
    P2: Provider<Optimism>,
    C: Clone + BlockHeaderCommit<OpBlockHeader>,
{
    /// Returns the [Commitment] used to validate the environment.
    pub fn commitment(&self) -> Commitment {
        self.commit
            .clone()
            .commit(self.inner.header(), self.inner.commitment().configID)
    }
}

impl<P2> HostOpEvmEnv<P2, DisputeGameCommit>
where
    P2: Provider<Optimism>,
{
    pub async fn into_input(self) -> Result<OpEvmInput> {
        // the inner environment has no specific commitment, so it will always return a block input
        let EvmInput::Block(input) = self.inner.into_input().await? else {
            unreachable!()
        };

        Ok(OpEvmInput::DisputeGame(ComposeInput::new(
            input,
            self.commit,
        )))
    }
}

/// Builder for building an [OpEvmEnv] on the host.
///
/// The builder can be created using [OpEvmEnv::builder()].
#[derive(Clone, Debug)]
pub struct OpEvmEnvBuilder<Stage, P2, Spec, G> {
    /// Underlying generic builder with no Beacon API config.
    inner: EvmEnvBuilder<P2, OpEvmFactory, Spec, ()>,
    /// Clone of the L2 provider.
    l2_provider: P2,
    /// Optional dispute game config.
    dispute_game_config: G,
    /// Stage of the builder.
    stage: PhantomData<Stage>,
}

/// First stage of a [OpEvmEnvBuilder] before a provider is set.
#[derive(Clone, Debug)]
pub struct PreProviderStage;
/// Second stage of a [OpEvmEnvBuilder] after a provider has been set.
#[derive(Clone, Debug)]
pub struct ProviderStage;

/// Configuration to commit to an OP dispute game.
#[derive(Clone, Debug)]
pub struct DisputeGameConfig<P1> {
    portal: OptimismPortal2<P1>,
    index: DisputeGameIndex,
}

// Callable with or without a provider and with or without a game.
impl<Stage, P2, Spec, G> OpEvmEnvBuilder<Stage, P2, Spec, G> {
    pub fn eip1186_proof_chunk_size(self, chunk_size: usize) -> Self {
        let Self {
            inner,
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        } = self;
        Self {
            inner: inner.eip1186_proof_chunk_size(chunk_size),
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        }
    }
}

// Callable without chain specification.
impl<Stage, P2, G> OpEvmEnvBuilder<Stage, P2, (), G> {
    /// Sets the [OpChainSpec].
    pub fn chain_spec(
        self,
        chain_spec: &OpChainSpec,
    ) -> OpEvmEnvBuilder<Stage, P2, &OpChainSpec, G> {
        OpEvmEnvBuilder {
            inner: self.inner.chain_spec(chain_spec),
            l2_provider: self.l2_provider,
            dispute_game_config: self.dispute_game_config,
            stage: self.stage,
        }
    }
}

// Callable only without a provider, only without a game.
impl<Spec> OpEvmEnvBuilder<PreProviderStage, (), Spec, ()> {
    /// Sets a fault dispute game that is feasible wrt the L1 `OptimismPortal` contract deployed at
    /// `portal`.
    ///
    /// This is used to create an [OpEvmInput] which can be validated against an L1 fault dispute
    /// game.
    pub fn dispute_game_from_rpc(
        self,
        portal: Address,
        l1_rpc: Url,
    ) -> OpEvmEnvBuilder<PreProviderStage, (), Spec, DisputeGameConfig<RootProvider<Ethereum>>>
    {
        self.dispute_game(portal, ProviderBuilder::default().connect_http(l1_rpc))
    }

    /// Sets a fault dispute game that is feasible wrt the L1 `OptimismPortal` contract deployed at
    /// `portal`.
    ///
    /// This is used to create an [OpEvmInput] which can be validated against an L1 fault dispute
    /// game.
    pub fn dispute_game<P1>(
        self,
        portal: Address,
        l1_provider: P1,
    ) -> OpEvmEnvBuilder<PreProviderStage, (), Spec, DisputeGameConfig<P1>>
    where
        P1: Provider<Ethereum>,
    {
        let Self {
            inner,
            l2_provider,
            stage,
            ..
        } = self;
        let dispute_game = DisputeGameConfig {
            portal: OptimismPortal2::new(portal, l1_provider),
            index: Default::default(),
        };

        OpEvmEnvBuilder {
            inner,
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        }
    }
}

// Callable only without a provider, with or without a game.
impl<G> OpEvmEnvBuilder<PreProviderStage, (), (), G> {
    /// Sets the L2 Optimism HTTP RPC endpoint that will be used by the [OpEvmEnv].
    pub fn rpc(self, url: Url) -> OpEvmEnvBuilder<ProviderStage, RootProvider<Optimism>, (), G> {
        self.provider(ProviderBuilder::default().connect_http(url))
    }

    /// Sets the L2 Optimism [Provider] that will be used by the [OpEvmEnv].
    pub fn provider<P2>(self, provider: P2) -> OpEvmEnvBuilder<ProviderStage, P2, (), G>
    where
        P2: Provider<Optimism> + Clone,
    {
        let inner = EvmEnv::builder().provider(provider.clone());
        let dispute_game = self.dispute_game_config;
        OpEvmEnvBuilder {
            inner,
            l2_provider: provider,
            dispute_game_config: dispute_game,
            stage: PhantomData,
        }
    }
}

// Callable only with a provider and only without a game.
impl<P2, Spec> OpEvmEnvBuilder<ProviderStage, P2, Spec, ()> {
    pub fn block_number(self, number: u64) -> Self {
        let Self {
            inner,
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        } = self;
        Self {
            inner: inner.block_number(number),
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        }
    }

    pub fn block_number_or_tag(self, block: BlockNumberOrTag) -> Self {
        let Self {
            inner,
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        } = self;
        Self {
            inner: inner.block_number_or_tag(block),
            l2_provider,
            dispute_game_config: dispute_game,
            stage,
        }
    }
}

impl<P2> OpEvmEnvBuilder<ProviderStage, P2, &OpChainSpec, ()> {
    pub async fn build(self) -> Result<HostOpEvmEnv<P2, ()>>
    where
        P2: Provider<Optimism>,
    {
        Ok(OpEvmEnv {
            inner: self.inner.build().await?,
            commit: (),
        })
    }
}

// Callable with or without a provider and only with a game.
impl<Stage, P1, P2, Spec> OpEvmEnvBuilder<Stage, P2, Spec, DisputeGameConfig<P1>> {
    pub fn game_index(mut self, index: DisputeGameIndex) -> Self {
        self.dispute_game_config.index = index;
        self
    }
}

// Callable only with a provider and with a game.
impl<P1, P2> OpEvmEnvBuilder<ProviderStage, P2, &OpChainSpec, DisputeGameConfig<P1>>
where
    P1: Provider<Ethereum>,
    P2: Provider<Optimism>,
{
    pub async fn build(self) -> Result<HostOpEvmEnv<P2, DisputeGameCommit>> {
        let game = self
            .dispute_game_config
            .portal
            .dispute_game(self.dispute_game_config.index, self.l2_provider)
            .await
            .context("failed to get dispute game from portal")?;

        let proof = game.output_root_proof;
        let env = self
            .inner
            .block_number(game.l2_block_number)
            .build()
            .await?;
        assert_eq!(proof.latestBlockhash, env.header().hash_slow());

        log::info!(
            "Committing to dispute game: rootClaim={},index={}",
            proof.hash(),
            game.index,
        );

        Ok(OpEvmEnv {
            inner: env,
            commit: DisputeGameCommit::new(game.index.to(), proof),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OutputRootProof, optimism::OP_MAINNET_CHAIN_SPEC};
    use alloy_primitives::address;
    use risc0_steel::Account;
    use test_log::test;

    const L1_URL: &str = "https://ethereum-rpc.publicnode.com";
    const L2_URL: &str = "https://optimism-rpc.publicnode.com";

    const OP_PORTAL_ADDRESS: Address = address!("bEb5Fc579115071764c7423A4f12eDde41f106Ed");

    #[test(tokio::test)]
    async fn clone_op_block_builder() {
        let builder = OpEvmEnv::builder()
            .rpc(L2_URL.parse().unwrap())
            .chain_spec(&OP_MAINNET_CHAIN_SPEC);
        // the builder should be cloneable
        let _ = builder.clone();
    }

    #[test(tokio::test)]
    #[cfg_attr(not(feature = "rpc-tests"), ignore = "RPC tests are disabled")]
    async fn build_op_block_env() {
        let builder = OpEvmEnv::builder()
            .rpc(L2_URL.parse().unwrap())
            .chain_spec(&OP_MAINNET_CHAIN_SPEC);
        let mut env = builder.build().await.unwrap();
        let _ = Account::preflight(Address::ZERO, &mut env).info().await;

        let host_commit = env.commitment();
        let input = env.into_input().await.unwrap();
        assert_eq!(
            input.into_env(&OP_MAINNET_CHAIN_SPEC).into_commitment(),
            host_commit
        );
    }

    #[test(tokio::test)]
    async fn clone_op_dispute_game_builder() {
        let builder = OpEvmEnv::builder()
            .dispute_game_from_rpc(OP_PORTAL_ADDRESS, L1_URL.parse().unwrap())
            .rpc(L2_URL.parse().unwrap())
            .game_index(DisputeGameIndex::Latest)
            .chain_spec(&OP_MAINNET_CHAIN_SPEC);
        // the builder should be cloneable
        let _ = builder.clone();
    }

    #[test(tokio::test)]
    #[cfg_attr(not(feature = "rpc-tests"), ignore = "RPC tests are disabled")]
    async fn build_op_dispute_game_env() {
        let builder = OpEvmEnv::builder()
            .rpc(L2_URL.parse().unwrap())
            .chain_spec(&OP_MAINNET_CHAIN_SPEC);
        let env = builder.build().await.unwrap();
        // mock an env with a dispute game commit, since building one requires an archive node
        let block_hash = env.header().seal();
        let mut env = HostOpEvmEnv {
            inner: env.inner,
            commit: DisputeGameCommit::new(
                u64::MAX,
                OutputRootProof {
                    version: Default::default(),
                    stateRoot: Default::default(),
                    messagePasserStorageRoot: Default::default(),
                    latestBlockhash: block_hash,
                },
            ),
        };
        let _ = Account::preflight(Address::ZERO, &mut env).info().await;

        let host_commit = env.commitment();
        let input = env.into_input().await.unwrap();
        assert_eq!(
            input.into_env(&OP_MAINNET_CHAIN_SPEC).into_commitment(),
            host_commit
        );
    }
}
