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

use crate::game::DisputeGameInput;
use alloy_consensus::{Eip658Value, TxReceipt};
use alloy_eips::{Encodable2718, Typed2718, eip4844, eip7691};
use alloy_evm::{Database, EvmFactory as AlloyEvmFactory};
use alloy_op_evm::OpEvmFactory as AlloyOpEvmFactory;
use alloy_primitives::{Address, B256, BlockNumber, Bloom, Bytes, ChainId, Sealable, TxKind, U256};
use alloy_rlp::BufMut;
use delegate::delegate;
use op_alloy_network::{Network, Optimism};
use op_revm::{OpTransaction, spec::OpSpecId};
use risc0_steel::{
    BlockInput, Commitment, EvmBlockHeader, EvmEnv, EvmFactory, EvmSpecId, StateDb,
    config::{ChainSpec, ForkCondition},
    revm::{
        context::{BlockEnv, CfgEnv, TxEnv},
        context_interface::block::BlobExcessGasAndPrice,
        inspector::NoOpInspector,
        primitives::hardfork::SpecId,
    },
    serde::{Eip2718Wrapper, RlpHeader},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, convert::Into, error::Error, sync::LazyLock};

#[cfg(feature = "host")]
mod host;

#[cfg(feature = "host")]
pub use host::*;

/// The OP Mainnet [ChainSpec].
pub static OP_MAINNET_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 10,
    forks: BTreeMap::from(
        [
            (OpSpecId::BEDROCK, ForkCondition::Block(105_235_063)),
            (OpSpecId::REGOLITH, ForkCondition::Timestamp(0)),
            (OpSpecId::CANYON, ForkCondition::Timestamp(1_704_992_401)),
            (OpSpecId::ECOTONE, ForkCondition::Timestamp(1_710_374_401)),
            (OpSpecId::FJORD, ForkCondition::Timestamp(1_720_627_201)),
            (OpSpecId::GRANITE, ForkCondition::Timestamp(1_726_070_401)),
            (OpSpecId::HOLOCENE, ForkCondition::Timestamp(1_736_445_601)),
            (OpSpecId::ISTHMUS, ForkCondition::Timestamp(1_746_806_401)),
            (OpSpecId::JOVIAN, ForkCondition::Timestamp(1_764_691_201)),
        ]
        .map(|(id, cond)| (id.into(), cond)),
    ),
});

/// The OP Sepolia [ChainSpec].
pub static OP_SEPOLIA_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 11155420,
    forks: BTreeMap::from(
        [
            (OpSpecId::BEDROCK, ForkCondition::Block(0)),
            (OpSpecId::REGOLITH, ForkCondition::Timestamp(0)),
            (OpSpecId::CANYON, ForkCondition::Timestamp(1_699_981_200)),
            (OpSpecId::ECOTONE, ForkCondition::Timestamp(1_708_534_800)),
            (OpSpecId::FJORD, ForkCondition::Timestamp(1_716_998_400)),
            (OpSpecId::GRANITE, ForkCondition::Timestamp(1_723_478_400)),
            (OpSpecId::HOLOCENE, ForkCondition::Timestamp(1_732_633_200)),
            (OpSpecId::ISTHMUS, ForkCondition::Timestamp(1_744_905_600)),
            (OpSpecId::JOVIAN, ForkCondition::Timestamp(1_763_568_001)),
        ]
        .map(|(id, cond)| (id.into(), cond)),
    ),
});

/// The Base Mainnet [ChainSpec].
pub static BASE_MAINNET_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 8453,
    forks: BTreeMap::from(
        [
            (OpSpecId::BEDROCK, ForkCondition::Block(0)),
            (OpSpecId::REGOLITH, ForkCondition::Timestamp(0)),
            (OpSpecId::CANYON, ForkCondition::Timestamp(1_704_992_401)),
            (OpSpecId::ECOTONE, ForkCondition::Timestamp(1_710_374_401)),
            (OpSpecId::FJORD, ForkCondition::Timestamp(1_720_627_201)),
            (OpSpecId::GRANITE, ForkCondition::Timestamp(1_726_070_401)),
            (OpSpecId::HOLOCENE, ForkCondition::Timestamp(1_736_445_601)),
            (OpSpecId::ISTHMUS, ForkCondition::Timestamp(1_746_806_401)),
            (OpSpecId::JOVIAN, ForkCondition::Timestamp(1_764_691_201)),
        ]
        .map(|(id, cond)| (id.into(), cond)),
    ),
});

/// The Base Sepolia [ChainSpec].
pub static BASE_SEPOLIA_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 84532,
    forks: OP_SEPOLIA_CHAIN_SPEC.forks.clone(),
});

/// [EvmFactory] for Optimism.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpEvmFactory;

impl EvmFactory for OpEvmFactory {
    type Evm<DB: Database> = <AlloyOpEvmFactory as AlloyEvmFactory>::Evm<DB, NoOpInspector>;
    type Tx = <AlloyOpEvmFactory as AlloyEvmFactory>::Tx;
    type Error<DBError: Error + Send + Sync + 'static> =
        <AlloyOpEvmFactory as AlloyEvmFactory>::Error<DBError>;
    type HaltReason = <AlloyOpEvmFactory as AlloyEvmFactory>::HaltReason;
    type Spec = <AlloyOpEvmFactory as AlloyEvmFactory>::Spec;
    type SpecId = OpEvmSpecId;
    type Header = OpBlockHeader;
    type Receipt = OpEvmReceipt;

    fn new_tx(address: Address, data: Bytes) -> Self::Tx {
        OpTransaction {
            base: TxEnv {
                caller: address,
                kind: TxKind::Call(address),
                data,
                chain_id: None,
                ..Default::default()
            },
            enveloped_tx: Some(Bytes::new()),
            ..Default::default()
        }
    }

    fn create_evm<DB: Database>(
        db: DB,
        chain_id: ChainId,
        spec_id: Self::SpecId,
        header: &Self::Header,
    ) -> Self::Evm<DB> {
        let mut cfg_env = CfgEnv::new_with_spec(spec_id.into()).with_chain_id(chain_id);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_balance_check = true;
        cfg_env.disable_block_gas_limit = true;
        // Disabled because eth_call is sometimes used with eoa senders
        cfg_env.disable_eip3607 = true;
        // The basefee should be ignored for eth_call
        cfg_env.disable_base_fee = true;

        let block_env = header.to_block_env(spec_id);

        AlloyOpEvmFactory::default().create_evm(db, (cfg_env, block_env).into())
    }
}

/// [ChainSpec] for Optimism.
pub type OpChainSpec = ChainSpec<OpEvmSpecId>;

/// [EvmFactory::SpecId] for Optimism.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OpEvmSpecId(OpSpecId);

impl From<OpSpecId> for OpEvmSpecId {
    fn from(spec: OpSpecId) -> Self {
        Self(spec)
    }
}

impl From<OpEvmSpecId> for OpSpecId {
    fn from(spec: OpEvmSpecId) -> Self {
        spec.0
    }
}
impl EvmSpecId for OpEvmSpecId {
    #[inline]
    fn has_eip4788(&self) -> bool {
        self.0 >= OpSpecId::ECOTONE
    }
    #[inline]
    fn has_eip2935(&self) -> bool {
        self.0 >= OpSpecId::ISTHMUS
    }
    #[inline]
    fn to_u32(&self) -> u32 {
        self.0 as u32
    }
}

type OpHeader = <Optimism as Network>::Header;

/// [EvmFactory::Header] for Optimism.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpBlockHeader(pub RlpHeader<OpHeader>);

impl Sealable for OpBlockHeader {
    delegate! {
        to self.0 { fn hash_slow(&self) -> B256; }
    }
}

impl EvmBlockHeader for OpBlockHeader {
    type SpecId = OpEvmSpecId;

    #[inline]
    fn parent_hash(&self) -> &B256 {
        &self.0.inner().parent_hash
    }
    #[inline]
    fn number(&self) -> BlockNumber {
        self.0.inner().number
    }
    #[inline]
    fn timestamp(&self) -> u64 {
        self.0.inner().timestamp
    }
    #[inline]
    fn state_root(&self) -> &B256 {
        &self.0.inner().state_root
    }
    #[inline]
    fn receipts_root(&self) -> &B256 {
        &self.0.inner().receipts_root
    }
    #[inline]
    fn logs_bloom(&self) -> &Bloom {
        &self.0.inner().logs_bloom
    }

    #[inline]
    fn to_block_env(&self, spec_id: Self::SpecId) -> BlockEnv {
        let header = self.0.inner();

        let eth_spec_id = spec_id.0.into_eth_spec();
        let blob_excess_gas_and_price =
            header
                .excess_blob_gas
                .map(|excess_blob_gas| match eth_spec_id {
                    SpecId::CANCUN => BlobExcessGasAndPrice::new(
                        excess_blob_gas,
                        eip4844::BLOB_GASPRICE_UPDATE_FRACTION as u64,
                    ),
                    SpecId::PRAGUE => BlobExcessGasAndPrice::new(
                        excess_blob_gas,
                        eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA as u64,
                    ),
                    SpecId::OSAKA => BlobExcessGasAndPrice::new(
                        excess_blob_gas,
                        eip7691::BLOB_GASPRICE_UPDATE_FRACTION_PECTRA as u64,
                    ),
                    _ => unimplemented!(
                        "unsupported spec with `excess_blob_gas`: {}",
                        <&'static str>::from(spec_id.0)
                    ),
                });

        BlockEnv {
            number: U256::from(header.number),
            beneficiary: header.beneficiary,
            timestamp: U256::from(header.timestamp),
            gas_limit: header.gas_limit,
            basefee: header.base_fee_per_gas.unwrap_or_default(),
            difficulty: header.difficulty,
            prevrandao: (spec_id.0 >= OpSpecId::BEDROCK).then_some(header.mix_hash),
            blob_excess_gas_and_price,
        }
    }
}

#[cfg(feature = "host")]
impl<H> TryFrom<alloy::rpc::types::Header<H>> for OpBlockHeader
where
    OpHeader: TryFrom<H>,
{
    type Error = <OpHeader as TryFrom<H>>::Error;

    #[inline]
    fn try_from(value: alloy::rpc::types::Header<H>) -> Result<Self, Self::Error> {
        Ok(Self(RlpHeader::new(value.inner.try_into()?)))
    }
}

/// [EvmFactory::Receipt] for Optimism.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpEvmReceipt(Eip2718Wrapper<<Optimism as Network>::ReceiptEnvelope>);

impl Typed2718 for OpEvmReceipt {
    delegate! {
        to self.0 { fn ty(&self) -> u8; }
    }
}

impl Encodable2718 for OpEvmReceipt {
    delegate! {
        to self.0 {
            fn encode_2718_len(&self) -> usize;
            fn encode_2718(&self, out: &mut dyn BufMut);
        }
    }
}

impl TxReceipt for OpEvmReceipt {
    type Log = <<Optimism as Network>::ReceiptEnvelope as TxReceipt>::Log;

    delegate! {
        to self.0 {
            fn status_or_post_state(&self) -> Eip658Value;
            fn status(&self) -> bool;
            fn bloom(&self) -> Bloom;
            fn cumulative_gas_used(&self) -> u64;
            fn logs(&self) -> &[Self::Log];
        }
    }
}

#[cfg(feature = "host")]
impl From<op_alloy_rpc_types::OpTransactionReceipt> for OpEvmReceipt {
    #[inline]
    fn from(rpc_receipt: op_alloy_rpc_types::OpTransactionReceipt) -> Self {
        // Unfortunately ReceiptResponse does not implement ReceiptEnvelope, so we have to
        // manually convert it.
        // TODO(https://github.com/alloy-rs/alloy/issues/854): use ReceiptEnvelope directly
        Self(Eip2718Wrapper::new(
            rpc_receipt.inner.into_inner().map_logs(Into::into),
        ))
    }
}

/// The serializable input to derive and validate an [EvmEnv] from.
#[non_exhaustive]
#[derive(Clone, Serialize, Deserialize)]
pub enum OpEvmInput {
    Block(BlockInput<OpEvmFactory>),
    DisputeGame(DisputeGameInput),
}

impl OpEvmInput {
    #[inline]
    pub fn into_env(self, chain_spec: &OpChainSpec) -> EvmEnv<StateDb, OpEvmFactory, Commitment> {
        match self {
            OpEvmInput::Block(input) => input.into_env(chain_spec),
            OpEvmInput::DisputeGame(input) => input.into_env(chain_spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::b256;

    mod op {
        use super::*;

        #[test]
        fn mainnet_spec_digest() {
            assert_eq!(
                OP_MAINNET_CHAIN_SPEC.digest(),
                b256!("0x6fa1d26e6f4adab901261db61a3b411ad7aebebc7027639d55a3b72cacc4a867")
            );
        }

        #[test]
        fn sepolia_spec_digest() {
            assert_eq!(
                OP_SEPOLIA_CHAIN_SPEC.digest(),
                b256!("0xb5a59c839834a212b03577274ce72572a97933fada4bf63b820173b87dc935c1")
            );
        }
    }

    mod base {
        use super::*;

        #[test]
        fn mainnet_spec_digest() {
            assert_eq!(
                BASE_MAINNET_CHAIN_SPEC.digest(),
                b256!("0xde0027ffd04b70b50fb52de9d6738b0dc66c1d84654ca3889b57f790979f6905")
            );
        }

        #[test]
        fn sepolia_spec_digest() {
            assert_eq!(
                BASE_SEPOLIA_CHAIN_SPEC.digest(),
                b256!("0xa2e376e0229be98aed684c4c52cb9f119f1757ac9d9e5b172a713908f8a3a739")
            );
        }
    }
}
