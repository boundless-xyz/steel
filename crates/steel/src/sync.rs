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

//! Synchronous contract-call interface for both host and guest environments.
//!
//! The [`SyncEnv`] trait provides a unified synchronous API for executing smart contract calls
//! that works identically in both the host (preflight) and guest (proof) contexts. This allows
//! users to write their contract interaction logic once and reuse it in both environments.
//!
//! # Example
//!
//! ```ignore
//! use alloy_primitives::{Address, U256};
//! use alloy_sol_types::sol;
//! use risc0_steel::{EvmBlockHeader, SyncEnv};
//!
//! sol! {
//!     interface IERC20 {
//!         function totalSupply() external view returns (uint256);
//!     }
//! }
//!
//! fn my_validation(env: &mut impl SyncEnv, contract: Address) {
//!     let supply: U256 = env.call(contract, &IERC20::totalSupplyCall {});
//!     assert!(supply > U256::ZERO);
//!     let ts = env.header().timestamp();
//!     // ...
//! }
//! ```

use alloy_primitives::{Address, Sealed};
use alloy_sol_types::SolCall;
use anyhow::Result;

use crate::{Contract, EvmBlockHeader, EvmFactory, GuestEvmEnv};

/// A synchronous contract-call interface for both host and guest environments.
///
/// This trait abstracts over the difference between guest (sync) and host (async) contract calls,
/// allowing shared validation logic to be written once as a generic function.
///
/// In the guest, calls are executed synchronously against the proven state.
/// On the host, calls are executed via [`Contract::preflight`] with async-to-sync bridging
/// using [`tokio::task::block_in_place`].
pub trait SyncEnv {
    /// The block header type, which dereferences to an [`EvmBlockHeader`].
    type Header: core::ops::Deref<Target: EvmBlockHeader>;

    /// Executes a contract call, returning a [`Result`].
    ///
    /// This is the core method that implementations must provide.
    fn try_call<S>(&mut self, addr: Address, call: &S) -> Result<S::Return>
    where
        S: SolCall + Send + Sync + 'static,
        S::Return: Send;

    /// Executes a contract call, panicking on failure.
    ///
    /// This is a convenience wrapper around [`SyncEnv::try_call`] that unwraps the result.
    fn call<S>(&mut self, addr: Address, call: &S) -> S::Return
    where
        S: SolCall + Send + Sync + 'static,
        S::Return: Send,
    {
        self.try_call(addr, call).unwrap()
    }

    /// Returns a reference to the sealed block header.
    fn header(&self) -> &Self::Header;
}

/// Guest implementation: always available.
///
/// Uses [`Contract::new`] and synchronous [`CallBuilder::try_call`] to execute calls
/// against the proven state database.
impl<F: EvmFactory> SyncEnv for GuestEvmEnv<F> {
    type Header = Sealed<F::Header>;

    fn try_call<S>(&mut self, addr: Address, call: &S) -> Result<S::Return>
    where
        S: SolCall + Send + Sync + 'static,
        S::Return: Send,
    {
        Ok(Contract::new(addr, self).call_builder(call).try_call()?)
    }

    fn header(&self) -> &Self::Header {
        // Calls the inherent EvmEnv::header method (inherent methods have priority).
        self.header()
    }
}

/// Host implementation: behind the `host` feature gate.
///
/// Uses [`Contract::preflight`] and bridges the async [`CallBuilder::call`] to synchronous
/// execution via [`tokio::task::block_in_place`] + [`tokio::runtime::Handle::block_on`].
///
/// `block_in_place` is used (rather than `spawn_blocking`) because `&mut self` is not `Send`.
/// This is the same pattern used by `reqwest::blocking` and other Rust libraries.
#[cfg(feature = "host")]
mod host_impl {
    use super::*;
    use crate::host::{HostEvmEnv, db::ProviderDb};
    use alloy::network::Network;
    use alloy::providers::Provider;

    impl<F, N, P, C> SyncEnv for HostEvmEnv<ProviderDb<N, P>, F, C>
    where
        F: EvmFactory,
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
    {
        type Header = Sealed<F::Header>;

        fn try_call<S>(&mut self, addr: Address, call: &S) -> Result<S::Return>
        where
            S: SolCall + Send + Sync + 'static,
            S::Return: Send,
        {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let mut contract = Contract::preflight(addr, self);
                    contract.call_builder(call).call().await
                })
            })
        }

        fn header(&self) -> &Self::Header {
            // Calls the inherent EvmEnv::header method (inherent methods have priority).
            self.header()
        }
    }
}
