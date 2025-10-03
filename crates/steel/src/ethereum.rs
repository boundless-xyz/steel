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
    config::{ChainSpec, ForkCondition},
    serde::{Eip2718Wrapper, RlpHeader},
    CallError, EvmBlockHeader, EvmEnv, EvmFactory, EvmInput, EvmSpecId,
};
use alloy_consensus::{Eip658Value, TxReceipt};
use alloy_eips::{eip4844, eip7691, Encodable2718, Typed2718};
use alloy_evm::{Database, EthEvmFactory as AlloyEthEvmFactory, EvmFactory as AlloyEvmFactory};
use alloy_primitives::{Address, BlockNumber, Bloom, Bytes, TxKind, B256, U256};
use delegate::delegate;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::block::BlobExcessGasAndPrice,
    inspector::NoOpInspector,
    primitives::hardfork::SpecId,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, error::Error, sync::LazyLock};

/// The Ethereum Sepolia [ChainSpec].
pub static ETH_SEPOLIA_CHAIN_SPEC: LazyLock<EthChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 11155111,
    forks: BTreeMap::from([
        (SpecId::MERGE, ForkCondition::Block(1735371)),
        (SpecId::SHANGHAI, ForkCondition::Timestamp(1677557088)),
        (SpecId::CANCUN, ForkCondition::Timestamp(1706655072)),
        (SpecId::PRAGUE, ForkCondition::Timestamp(1741159776)),
    ]),
});

/// The Ethereum Holešky [ChainSpec].
pub static ETH_HOLESKY_CHAIN_SPEC: LazyLock<EthChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 17000,
    forks: BTreeMap::from([
        (SpecId::MERGE, ForkCondition::Block(0)),
        (SpecId::SHANGHAI, ForkCondition::Timestamp(1696000704)),
        (SpecId::CANCUN, ForkCondition::Timestamp(1707305664)),
        (SpecId::PRAGUE, ForkCondition::Timestamp(1740434112)),
    ]),
});

/// The Ethereum Mainnet [ChainSpec].
pub static ETH_MAINNET_CHAIN_SPEC: LazyLock<EthChainSpec> = LazyLock::new(|| ChainSpec {
    chain_id: 1,
    forks: BTreeMap::from([
        (SpecId::MERGE, ForkCondition::Block(15537394)),
        (SpecId::SHANGHAI, ForkCondition::Timestamp(1681338455)),
        (SpecId::CANCUN, ForkCondition::Timestamp(1710338135)),
        (SpecId::PRAGUE, ForkCondition::Timestamp(1746612311)),
    ]),
});

/// [ChainSpec] for a custom Steel Testnet using the Prague EVM.
pub static STEEL_TEST_PRAGUE_CHAIN_SPEC: LazyLock<ChainSpec<SpecId>> =
    LazyLock::new(|| ChainSpec::new_single(5733100018, SpecId::PRAGUE));

/// [EvmFactory] for Ethereum.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EthEvmFactory;

impl EvmFactory for EthEvmFactory {
    type Evm<DB: Database> = <AlloyEthEvmFactory as AlloyEvmFactory>::Evm<DB, NoOpInspector>;
    type Tx = <AlloyEthEvmFactory as AlloyEvmFactory>::Tx;
    type Error<DBError: Error + Send + Sync + 'static> =
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

/// [CallError] for Ethereum.
pub type EthCallError = CallError<<EthEvmFactory as EvmFactory>::HaltReason>;

/// [ChainSpec] for Ethereum.
pub type EthChainSpec = ChainSpec<SpecId>;

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

/// [EvmEnv] for Ethereum.
pub type EthEvmEnv<D, C> = EvmEnv<D, EthEvmFactory, C>;

/// [EvmInput] for Ethereum.
pub type EthEvmInput = EvmInput<EthEvmFactory>;

/// [EvmBlockHeader] for Ethereum.
pub type EthBlockHeader = RlpHeader<alloy_consensus::Header>;

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
    use alloy::primitives::b256;

    use super::{
        ETH_HOLESKY_CHAIN_SPEC, ETH_MAINNET_CHAIN_SPEC, ETH_SEPOLIA_CHAIN_SPEC,
        STEEL_TEST_PRAGUE_CHAIN_SPEC,
    };

    // NOTE: If these are updated here, make sure to update them in Steel.sol

    #[test]
    fn mainnet_spec_digest() {
        assert_eq!(
            ETH_MAINNET_CHAIN_SPEC.digest(),
            b256!("0x9a223c7ca04c969f1cacbe5b8db44c308b2c53390505d3d48c834ed4469fc839")
        );
    }

    #[test]
    fn sepolia_spec_digest() {
        assert_eq!(
            ETH_SEPOLIA_CHAIN_SPEC.digest(),
            b256!("0x5c9552dc9bfad8572ded4f818bb35b0f4260660c1554236986b768ae999b4b60")
        );
    }

    #[test]
    fn holesky_spec_digest() {
        assert_eq!(
            ETH_HOLESKY_CHAIN_SPEC.digest(),
            b256!("0x8eae1ba5f877e6ad7007bf6985f5245be7d758457fb4eb7e6a72d47f49bea389")
        );
    }

    #[test]
    fn testnet_spec_digest() {
        assert_eq!(
            STEEL_TEST_PRAGUE_CHAIN_SPEC.digest(),
            b256!("0x33e32d9590cd4b168773ca27de65d535f2e744274b1437acb712dd4264f2eb87")
        );
    }
}
