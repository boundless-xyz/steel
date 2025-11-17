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

//! Wrappers for interacting with Ethereum precompiled contracts.

use crate::{Contract, EvmFactory, GuestEvmEnv};
use alloy_eips::{eip2935, eip4788};
use alloy_primitives::{B256, U256};
use alloy_sol_types::SolValue;

/// A typed wrapper for interacting with the EIP-4788 Beacon Roots contract.
///
/// This provides a convenient way to query for historical beacon chain roots within the guest.
pub struct BeaconRootsContract<E>(Contract<E>);

impl<'a, F: EvmFactory> BeaconRootsContract<&'a GuestEvmEnv<F>> {
    /// Creates a new `BeaconRootsContract` instance for use within the guest.
    pub fn new(env: &'a GuestEvmEnv<F>) -> Self {
        Self(Contract::new(eip4788::BEACON_ROOTS_ADDRESS, env))
    }

    /// Calls the beacon roots contract with the given `timestamp` and returns the corresponding
    /// beacon root.
    ///
    /// This function will panic if the contract call reverts or fails.
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

/// A typed wrapper for interacting with the EIP-2935 History Storage contract.
///
/// This provides a convenient way to query for historical block hashes within the guest.
pub struct HistoryStorageContract<E>(Contract<E>);

impl<'a, F: EvmFactory> HistoryStorageContract<&'a GuestEvmEnv<F>> {
    /// Creates a new `HistoryStorageContract` instance for use within the guest.
    pub fn new(env: &'a GuestEvmEnv<F>) -> Self {
        Self(Contract::new(eip2935::HISTORY_STORAGE_ADDRESS, env))
    }

    /// Calls the history storage contract with the given `block_number` and returns the
    /// corresponding execution block hash.
    ///
    /// This function will panic if the contract call reverts or fails.
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
        Contract,
        host::{HostEvmEnv, db::ProviderDb},
    };
    use alloy::{network::Network, providers::Provider};
    use alloy_sol_types::SolValue;
    use anyhow::Context;

    /// Creates a new `BeaconRootsContract` instance for use on the host for preflighting.
    impl<'a, N, P, F, C> BeaconRootsContract<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
        F: EvmFactory,
    {
        pub fn preflight(env: &'a mut HostEvmEnv<ProviderDb<N, P>, F, C>) -> Self {
            Self(Contract::preflight(eip4788::BEACON_ROOTS_ADDRESS, env))
        }

        /// Preflights a call to the beacon roots contract with the given `timestamp`.
        pub async fn call(&mut self, timestamp: U256) -> anyhow::Result<B256> {
            let resp = self.0.raw(timestamp.abi_encode().into()).call().await?;
            B256::abi_decode_validate(&resp).context("failed to decode return data")
        }
    }

    impl<'a, N, P, F, C> HistoryStorageContract<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
        F: EvmFactory,
    {
        /// Creates a new `HistoryStorageContract` instance for use on the host for preflighting.
        pub fn preflight(env: &'a mut HostEvmEnv<ProviderDb<N, P>, F, C>) -> Self {
            Self(Contract::preflight(eip2935::HISTORY_STORAGE_ADDRESS, env))
        }

        /// Preflights a call to the history storage contract with the given `block_number`.
        pub async fn call(&mut self, block_number: U256) -> anyhow::Result<B256> {
            let resp = self.0.raw(block_number.abi_encode().into()).call().await?;
            B256::abi_decode_validate(&resp).context("failed to decode return data")
        }
    }
}
