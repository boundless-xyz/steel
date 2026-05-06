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

use crate::game::DisputeGameInput;
use alloy_consensus::{Eip658Value, ReceiptWithBloom, TxReceipt};
use alloy_eips::{Encodable2718, Typed2718, eip4844, eip7691};
use alloy_evm::{Database, EvmFactory as AlloyEvmFactory};
use alloy_op_evm::{OpEvmFactory as AlloyOpEvmFactory, OpTx};
use alloy_primitives::{Address, B256, BlockNumber, Bloom, Bytes, ChainId, Sealable, TxKind, U256};
use alloy_rlp::BufMut;
use delegate::delegate;
use op_alloy_consensus::OpReceipt;
use op_revm::OpTransaction;
use risc0_steel::{
    BlockInput, CallError, Commitment, EvmBlockHeader, EvmEnv, EvmFactory, StateDb,
    config::{ChainSpec, ForkCondition as FC},
    revm::{
        context::{BlockEnv, CfgEnv, TxEnv},
        context_interface::block::BlobExcessGasAndPrice,
        inspector::NoOpInspector,
        primitives::hardfork::SpecId,
    },
    serde::{Eip2718Wrapper, RlpHeader},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, error::Error, sync::LazyLock};

mod spec;
pub use spec::{OpRevmSpecId, OpSpecId, base};

#[cfg(feature = "host")]
mod host;

#[cfg(feature = "host")]
pub use host::*;

/// Lifts an array of `(OpRevmSpecId, FC)` chain-spec entries into the `(OpSpecId, FC)`
/// form `ChainSpec` expects. Lets chain-spec literals stay one line per fork without
/// scattering `.into()` calls across each entry.
fn forks<const N: usize>(entries: [(OpRevmSpecId, FC); N]) -> [(OpSpecId, FC); N] {
    entries.map(|(spec, cond)| (spec.into(), cond))
}

/// The OP Mainnet [ChainSpec].
pub static OP_MAINNET_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 10,
    forks: BTreeMap::from(forks([
        (OpRevmSpecId::BEDROCK, FC::Block(105_235_063)),
        (OpRevmSpecId::REGOLITH, FC::Timestamp(0)),
        (OpRevmSpecId::CANYON, FC::Timestamp(1_704_992_401)),
        (OpRevmSpecId::ECOTONE, FC::Timestamp(1_710_374_401)),
        (OpRevmSpecId::FJORD, FC::Timestamp(1_720_627_201)),
        (OpRevmSpecId::GRANITE, FC::Timestamp(1_726_070_401)),
        (OpRevmSpecId::HOLOCENE, FC::Timestamp(1_736_445_601)),
        (OpRevmSpecId::ISTHMUS, FC::Timestamp(1_746_806_401)),
        (OpRevmSpecId::JOVIAN, FC::Timestamp(1_764_691_201)),
    ])),
});

/// The OP Sepolia [ChainSpec].
pub static OP_SEPOLIA_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 11155420,
    forks: BTreeMap::from(forks([
        (OpRevmSpecId::BEDROCK, FC::Block(0)),
        (OpRevmSpecId::REGOLITH, FC::Timestamp(0)),
        (OpRevmSpecId::CANYON, FC::Timestamp(1_699_981_200)),
        (OpRevmSpecId::ECOTONE, FC::Timestamp(1_708_534_800)),
        (OpRevmSpecId::FJORD, FC::Timestamp(1_716_998_400)),
        (OpRevmSpecId::GRANITE, FC::Timestamp(1_723_478_400)),
        (OpRevmSpecId::HOLOCENE, FC::Timestamp(1_732_633_200)),
        (OpRevmSpecId::ISTHMUS, FC::Timestamp(1_744_905_600)),
        (OpRevmSpecId::JOVIAN, FC::Timestamp(1_763_568_001)),
    ])),
});

/// The Base Mainnet [ChainSpec].
pub static BASE_MAINNET_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 8453,
    forks: BTreeMap::from(forks([
        (OpRevmSpecId::BEDROCK, FC::Block(0)),
        (OpRevmSpecId::REGOLITH, FC::Timestamp(0)),
        (OpRevmSpecId::CANYON, FC::Timestamp(1_704_992_401)),
        (OpRevmSpecId::ECOTONE, FC::Timestamp(1_710_374_401)),
        (OpRevmSpecId::FJORD, FC::Timestamp(1_720_627_201)),
        (OpRevmSpecId::GRANITE, FC::Timestamp(1_726_070_401)),
        (OpRevmSpecId::HOLOCENE, FC::Timestamp(1_736_445_601)),
        (OpRevmSpecId::ISTHMUS, FC::Timestamp(1_746_806_401)),
        (OpRevmSpecId::JOVIAN, FC::Timestamp(1_764_691_201)),
        // Base names this fork "Azul"; EVM-level it's the Karst-equivalent
        // (Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951 P256VERIFY).
        (base::AZUL, FC::Timestamp(1_779_386_400)),
    ])),
});

/// The Base Sepolia [ChainSpec].
pub static BASE_SEPOLIA_CHAIN_SPEC: LazyLock<OpChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 84532,
    forks: BTreeMap::from(forks([
        (OpRevmSpecId::BEDROCK, FC::Block(0)),
        (OpRevmSpecId::REGOLITH, FC::Timestamp(0)),
        (OpRevmSpecId::CANYON, FC::Timestamp(1_699_981_200)),
        (OpRevmSpecId::ECOTONE, FC::Timestamp(1_708_534_800)),
        (OpRevmSpecId::FJORD, FC::Timestamp(1_716_998_400)),
        (OpRevmSpecId::GRANITE, FC::Timestamp(1_723_478_400)),
        (OpRevmSpecId::HOLOCENE, FC::Timestamp(1_732_633_200)),
        (OpRevmSpecId::ISTHMUS, FC::Timestamp(1_744_905_600)),
        (OpRevmSpecId::JOVIAN, FC::Timestamp(1_763_568_001)),
        // Base names this fork "Azul"; EVM-level it's the Karst-equivalent
        // (Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951 P256VERIFY).
        (base::AZUL, FC::Timestamp(1_776_708_000)),
    ])),
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
    type SpecId = OpSpecId;
    type Header = OpBlockHeader;
    type Receipt = OpEvmReceipt;

    fn new_tx(address: Address, data: Bytes) -> Self::Tx {
        OpTx(OpTransaction {
            base: TxEnv {
                caller: address,
                kind: TxKind::Call(address),
                data,
                chain_id: None,
                ..Default::default()
            },
            enveloped_tx: Some(Bytes::new()),
            ..Default::default()
        })
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

/// [CallError] for Optimism.
pub type OpCallError = CallError<<OpEvmFactory as EvmFactory>::HaltReason>;

/// [ChainSpec] for Optimism.
pub type OpChainSpec = ChainSpec<OpSpecId>;

type OpHeader = alloy_consensus::Header;

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
    type SpecId = OpSpecId;

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

        let eth_spec_id = spec_id.into_eth_spec();
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
                    _ => unimplemented!("unsupported spec with `excess_blob_gas`: {spec_id}"),
                });

        BlockEnv {
            number: U256::from(header.number),
            beneficiary: header.beneficiary,
            timestamp: U256::from(header.timestamp),
            gas_limit: header.gas_limit,
            basefee: header.base_fee_per_gas.unwrap_or_default(),
            difficulty: header.difficulty,
            prevrandao: (spec_id.into_inner() >= OpRevmSpecId::BEDROCK).then_some(header.mix_hash),
            blob_excess_gas_and_price,
            slot_num: header.slot_number.unwrap_or_default(),
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
pub struct OpEvmReceipt(Eip2718Wrapper<ReceiptWithBloom<OpReceipt>>);

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
    type Log = <ReceiptWithBloom<OpReceipt> as TxReceipt>::Log;

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

        let (receipt, bloom) = rpc_receipt.inner.into_inner().into_components();
        let eip2718_envelope = ReceiptWithBloom::new(receipt.map_logs(Into::into), bloom);

        Self(Eip2718Wrapper::new(eip2718_envelope))
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
                b256!("0xd16332d74ced8e4fa1cc0810be6d01ea607018745cba80c230a4b79498c513ef")
            );
        }

        #[test]
        fn sepolia_spec_digest() {
            assert_eq!(
                BASE_SEPOLIA_CHAIN_SPEC.digest(),
                b256!("0x3519660d6ecbd34367740f5ca18449cba8b389594f69f177bbf21c46e505c61e")
            );
        }
    }
}
