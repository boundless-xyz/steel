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

use crate::{Contract, EvmFactory, GuestEvmEnv};
use alloy_eips::{eip2935, eip4788};
use alloy_primitives::{B256, U256};
use alloy_sol_types::SolValue;

pub struct BeaconRootsContract<E>(Contract<E>);

impl<'a, F: EvmFactory> BeaconRootsContract<&'a GuestEvmEnv<F>> {
    pub fn new(env: &'a GuestEvmEnv<F>) -> Self {
        Self(Contract::new(eip4788::BEACON_ROOTS_ADDRESS, env))
    }

    pub fn call(self, block_number: U256) -> B256 {
        let resp = self
            .0
            .raw(block_number.abi_encode().into())
            .try_call()
            .expect("Executing beacon roots contract failed");
        B256::abi_decode_validate(&resp)
            .expect("Failed to decode return data, expected type 'Bytes32'")
    }
}

pub struct HistoryStorageContract<E>(Contract<E>);

impl<'a, F: EvmFactory> HistoryStorageContract<&'a GuestEvmEnv<F>> {
    pub fn new(env: &'a GuestEvmEnv<F>) -> Self {
        Self(Contract::new(eip2935::HISTORY_STORAGE_ADDRESS, env))
    }

    pub fn call(self, block_number: U256) -> B256 {
        let resp = self
            .0
            .raw(block_number.abi_encode().into())
            .try_call()
            .expect("Executing history storage contract failed");
        B256::abi_decode_validate(&resp)
            .expect("Failed to decode return data, expected type 'Bytes32'")
    }
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use crate::{
        contract::RawCall,
        host::{db::ProviderDb, HostEvmEnv},
        CallBuilder, Contract,
    };
    use alloy::{network::Network, providers::Provider};
    use alloy_sol_types::SolValue;

    impl<'a, N, P, F, C> BeaconRootsContract<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
        F: EvmFactory,
    {
        pub fn preflight(env: &'a mut HostEvmEnv<ProviderDb<N, P>, F, C>) -> Self {
            Self(Contract::preflight(eip4788::BEACON_ROOTS_ADDRESS, env))
        }

        pub async fn call(&mut self, timestamp: U256) -> anyhow::Result<B256> {
            let resp = self.0.raw(timestamp.abi_encode().into()).call().await?;
            Ok(B256::abi_decode_validate(&resp)?)
        }
    }

    impl<'a, N, P, F, C> HistoryStorageContract<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
        F: EvmFactory,
    {
        pub fn preflight(env: &'a mut HostEvmEnv<ProviderDb<N, P>, F, C>) -> Self {
            Self(Contract::preflight(eip2935::HISTORY_STORAGE_ADDRESS, env))
        }

        pub fn call_builder(
            &mut self,
            block_number: U256,
        ) -> CallBuilder<F::Tx, RawCall, &mut HostEvmEnv<ProviderDb<N, P>, F, C>> {
            self.0.raw(block_number.abi_encode().into())
        }

        pub async fn call(&mut self, block_number: U256) -> anyhow::Result<B256> {
            let resp = self.0.raw(block_number.abi_encode().into()).call().await?;
            Ok(B256::abi_decode_validate(&resp)?)
        }
    }
}
