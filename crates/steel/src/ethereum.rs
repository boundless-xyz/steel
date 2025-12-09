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

//! Type aliases and specifications for Ethereum.
use crate::{
    CallError, EvmBlockHeader, EvmEnv, EvmFactory, EvmInput, EvmSpecId,
    config::{ChainSpec, ForkCondition},
    serde::{Eip2718Wrapper, RlpHeader},
};
use alloy_consensus::{Eip658Value, TxReceipt};
use alloy_eips::{Encodable2718, Typed2718, eip4844, eip7691};
use alloy_evm::{Database, EthEvmFactory as AlloyEthEvmFactory, EvmFactory as AlloyEvmFactory};
use alloy_primitives::{Address, B256, BlockNumber, Bloom, Bytes, TxKind, U256};
use delegate::delegate;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::block::BlobExcessGasAndPrice,
    inspector::NoOpInspector,
    primitives::hardfork::SpecId,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, error::Error as StdError, sync::LazyLock};

/// [ChainSpec] for Ethereum.
pub type EthChainSpec = ChainSpec<SpecId>;

/// [CallError] for Ethereum.
pub type EthCallError = CallError<<EthEvmFactory as EvmFactory>::HaltReason>;

/// [EvmEnv] for Ethereum.
pub type EthEvmEnv<D, C> = EvmEnv<D, EthEvmFactory, C>;

/// [EvmInput] for Ethereum.
pub type EthEvmInput = EvmInput<EthEvmFactory>;

/// [EvmBlockHeader] for Ethereum.
pub type EthBlockHeader = RlpHeader<alloy_consensus::Header>;

macro_rules! define_chain_specs {
    ($($(#[$meta:meta])* $name:ident { chain_id: $id:literal, forks: $forks:expr $(,)? })*) => {
        $(
            $(#[$meta])*
            pub static $name: LazyLock<EthChainSpec> = LazyLock::new(|| EthChainSpec {
                chain_id: $id,
                forks: BTreeMap::from($forks),
            });
        )*

        impl EthChainSpec {
            /// Resolves a chain ID to a known [ChainSpec] reference.
            #[must_use]
            pub fn from_chain_id(chain_id: u64) -> Option<&'static Self> {
                match chain_id {
                    $($id => Some(&$name),)*
                    _ => None,
                }
            }
        }
    };
}

define_chain_specs! {
    /// The Ethereum Mainnet [ChainSpec].
    ETH_MAINNET_CHAIN_SPEC {
        chain_id: 1,
        forks: [
            (SpecId::MERGE, ForkCondition::Block(15_537_394)),
            (SpecId::SHANGHAI, ForkCondition::Timestamp(1_681_338_455)),
            (SpecId::CANCUN, ForkCondition::Timestamp(1_710_338_135)),
            (SpecId::PRAGUE, ForkCondition::Timestamp(1_746_612_311)),
            (SpecId::OSAKA, ForkCondition::Timestamp(1_764_798_551)),
        ],
    }
    /// The Ethereum Sepolia [ChainSpec].
    ETH_SEPOLIA_CHAIN_SPEC {
        chain_id: 11155111,
        forks: [
            (SpecId::MERGE, ForkCondition::Block(1_735_371)),
            (SpecId::SHANGHAI, ForkCondition::Timestamp(1_677_557_088)),
            (SpecId::CANCUN, ForkCondition::Timestamp(1_706_655_072)),
            (SpecId::PRAGUE, ForkCondition::Timestamp(1_741_159_776)),
            (SpecId::OSAKA, ForkCondition::Timestamp(1_760_427_360)),
        ],
    }
    /// The Ethereum Hoodi [ChainSpec].
    ETH_HOODI_CHAIN_SPEC {
        chain_id: 560048,
        forks: [
            (SpecId::CANCUN, ForkCondition::Block(0)),
            (SpecId::PRAGUE, ForkCondition::Timestamp(1_742_999_832)),
            (SpecId::OSAKA, ForkCondition::Timestamp(1_761_677_592)),
        ],
    }
    /// [ChainSpec] for a custom Steel Testnet using the Prague EVM.
    STEEL_TEST_PRAGUE_CHAIN_SPEC {
        chain_id: 5733100018,
        forks: [(SpecId::PRAGUE, ForkCondition::Block(0))],
    }
    /// [ChainSpec] for a custom Steel Testnet using the Osaka EVM.
    STEEL_TEST_OSAKA_CHAIN_SPEC {
        chain_id: 5733100019,
        forks: [(SpecId::OSAKA, ForkCondition::Block(0))],
    }
}

/// [EvmFactory] for Ethereum.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EthEvmFactory;

impl EvmFactory for EthEvmFactory {
    type Evm<DB: Database> = <AlloyEthEvmFactory as AlloyEvmFactory>::Evm<DB, NoOpInspector>;
    type Tx = <AlloyEthEvmFactory as AlloyEvmFactory>::Tx;
    type Error<DBError: StdError + Send + Sync + 'static> =
        <AlloyEthEvmFactory as AlloyEvmFactory>::Error<DBError>;
    type HaltReason = <AlloyEthEvmFactory as AlloyEvmFactory>::HaltReason;
    type Spec = <AlloyEthEvmFactory as AlloyEvmFactory>::Spec;
    type SpecId = SpecId;
    type Header = EthBlockHeader;
    type Receipt = EthReceipt;

    fn new_tx(address: Address, data: Bytes) -> Self::Tx {
        TxEnv {
            caller: address,
            kind: TxKind::Call(address),
            data,
            chain_id: None,
            ..Default::default()
        }
    }

    fn create_evm<DB: Database>(
        db: DB,
        chain_id: u64,
        spec_id: SpecId,
        header: &Self::Header,
    ) -> Self::Evm<DB> {
        let mut cfg_env = CfgEnv::new_with_spec(spec_id).with_chain_id(chain_id);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_balance_check = true;
        cfg_env.disable_block_gas_limit = true;
        // Disabled because eth_call is sometimes used with eoa senders
        cfg_env.disable_eip3607 = true;
        // The basefee should be ignored for eth_call
        cfg_env.disable_base_fee = true;

        let block_env = header.to_block_env(spec_id);

        AlloyEthEvmFactory::default().create_evm(db, (cfg_env, block_env).into())
    }
}

impl EvmSpecId for SpecId {
    #[inline]
    fn has_eip4788(&self) -> bool {
        self >= &SpecId::CANCUN
    }
    #[inline]
    fn has_eip2935(&self) -> bool {
        self >= &SpecId::PRAGUE
    }
    #[inline]
    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

impl EvmBlockHeader for EthBlockHeader {
    type SpecId = SpecId;

    #[inline]
    fn parent_hash(&self) -> &B256 {
        &self.inner().parent_hash
    }
    #[inline]
    fn number(&self) -> BlockNumber {
        self.inner().number
    }
    #[inline]
    fn timestamp(&self) -> u64 {
        self.inner().timestamp
    }
    #[inline]
    fn state_root(&self) -> &B256 {
        &self.inner().state_root
    }
    #[inline]
    fn receipts_root(&self) -> &B256 {
        &self.inner().receipts_root
    }
    #[inline]
    fn logs_bloom(&self) -> &Bloom {
        &self.inner().logs_bloom
    }

    #[inline]
    fn to_block_env(&self, spec: Self::SpecId) -> BlockEnv {
        let header = self.inner();

        let blob_excess_gas_and_price = header.excess_blob_gas.map(|excess_blob_gas| match spec {
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
            _ => unimplemented!("unsupported spec with `excess_blob_gas`: {spec}"),
        });

        BlockEnv {
            number: U256::from(header.number),
            beneficiary: header.beneficiary,
            timestamp: U256::from(header.timestamp),
            gas_limit: header.gas_limit,
            basefee: header.base_fee_per_gas.unwrap_or_default(),
            difficulty: header.difficulty,
            prevrandao: (spec >= SpecId::MERGE).then_some(header.mix_hash),
            blob_excess_gas_and_price,
        }
    }
}

/// [EvmFactory::Receipt] for Ethereum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EthReceipt(Eip2718Wrapper<alloy_consensus::ReceiptEnvelope>);

impl Typed2718 for EthReceipt {
    delegate! {
        to self.0 { fn ty(&self) -> u8; }
    }
}

impl Encodable2718 for EthReceipt {
    delegate! {
        to self.0 {
            fn encode_2718_len(&self) -> usize;
            fn encode_2718(&self, out: &mut dyn alloy_rlp::BufMut);
        }
    }
}

impl TxReceipt for EthReceipt {
    type Log = <alloy_consensus::ReceiptEnvelope as TxReceipt>::Log;

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
impl From<alloy_rpc_types::TransactionReceipt> for EthReceipt {
    #[inline]
    fn from(rpc_receipt: alloy_rpc_types::TransactionReceipt) -> Self {
        // Unfortunately ReceiptResponse does not implement ReceiptEnvelope, so we have to
        // manually convert it.
        // TODO(https://github.com/alloy-rs/alloy/issues/854): use ReceiptEnvelope directly
        Self(Eip2718Wrapper::new(
            rpc_receipt.into_inner().into_primitives_receipt(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_chain_id() {
        assert!(EthChainSpec::from_chain_id(0).is_none());
        assert_eq!(
            EthChainSpec::from_chain_id(1),
            Some(&*ETH_MAINNET_CHAIN_SPEC)
        );
    }

    mod spec_digest {
        use super::*;
        use alloy::primitives::b256;

        // NOTE: If these are updated here, make sure to update them in Steel.sol

        #[test]
        fn mainnet() {
            assert_eq!(
                ETH_MAINNET_CHAIN_SPEC.digest(),
                b256!("0x47dc59f84afd2e9e7a48c4012004ab7c77fbd9acf822bf1143b8442c6c8851d4")
            );
        }

        #[test]
        fn sepolia() {
            assert_eq!(
                ETH_SEPOLIA_CHAIN_SPEC.digest(),
                b256!("0x90c1e882b1f0fda4dc7f1c66c07ed3d2a74e443834905faa9f32f583b71f459d")
            );
        }

        #[test]
        fn hoodi() {
            assert_eq!(
                ETH_HOODI_CHAIN_SPEC.digest(),
                b256!("0x34cb1defd939572b00439d2c13f93c033b82227067371c910ad104d527c78860")
            );
        }

        #[test]
        fn testnet_prague() {
            assert_eq!(
                STEEL_TEST_PRAGUE_CHAIN_SPEC.digest(),
                b256!("0x33e32d9590cd4b168773ca27de65d535f2e744274b1437acb712dd4264f2eb87")
            );
        }

        #[test]
        fn testnet_osaka() {
            assert_eq!(
                STEEL_TEST_OSAKA_CHAIN_SPEC.digest(),
                b256!("0x2a80c688d324f578513161dda9e9a5773c0ee052f50304a94339e966da28b2ad")
            );
        }
    }
}
