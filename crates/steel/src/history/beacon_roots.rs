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

use super::state::Error;
use crate::history::SingleContractState;
use alloy_primitives::{Address, B256, U256, address, b256, uint};
use revm::Database;

/// Address where the EIP-4788 beacon roots contract is deployed.
pub const ADDRESS: Address = address!("0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02");
/// The length of the buffer that stores historical entries, i.e., the number of stored
/// timestamps and roots.
pub const HISTORY_BUFFER_LENGTH: U256 = uint!(8191_U256);

/// Hash of the deployed EVM bytecode.
const CODE_HASH: B256 = b256!("f57acd40259872606d76197ef052f3d35588dadf919ee1f0e3cb9b62d3f4b02c");

/// The `BeaconRootsContract` is responsible for storing and retrieving historical beacon roots.
///
/// It is an exact reimplementation of the beacon roots contract as defined in [EIP-4788](https://eips.ethereum.org/EIPS/eip-4788).
/// It is deployed at the address `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02` and has the
/// following storage layout:
/// - `timestamp_idx = timestamp % HISTORY_BUFFER_LENGTH`: Stores the timestamp at this index.
/// - `root_idx = timestamp_idx + HISTORY_BUFFER_LENGTH`: Stores the beacon root at this index.
pub struct BeaconRootsContract<D> {
    db: D,
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use crate::{history::SingleContractState, host::db::ProviderDb};
    use alloy::providers::{Network, Provider};
    use anyhow::{Context, anyhow, ensure};

    impl<N, P> BeaconRootsContract<ProviderDb<N, P>>
    where
        N: Network,
        P: Provider<N>,
    {
        /// Creates a new instance of the `ExecutionHashContract` from the given db.
        pub fn preflight(db: ProviderDb<N, P>) -> Self {
            Self { db }
        }

        /// Returns the timestamp stored in the slot which corresponds to the given `timestamp`.
        pub async fn get_timestamp(&self, timestamp: U256) -> anyhow::Result<U256> {
            // compute the key of the storage slot
            let timestamp_idx = timestamp % HISTORY_BUFFER_LENGTH;
            // return its value
            self.db
                .provider()
                .get_storage_at(ADDRESS, timestamp_idx)
                .hash(self.db.block())
                .await
                .context("eth_getStorageAt failed")
        }

        /// Prepares a [SingleContractState] by retrieving the beacon root from an RPC provider and
        /// constructing the necessary proofs.
        ///
        /// It fetches the minimal set of Merkle proofs (for the contract's state and storage)
        /// required to verify and retrieve the beacon root associated with the given
        /// `timestamp`.
        pub async fn get(&self, timestamp: U256) -> anyhow::Result<(B256, SingleContractState)> {
            // compute the keys of the two storage slots that will be accessed
            let timestamp_idx = timestamp % HISTORY_BUFFER_LENGTH;
            let root_idx = timestamp_idx + HISTORY_BUFFER_LENGTH;

            // derive the minimal state needed to query and validate
            let proof = self
                .db
                .get_proof(ADDRESS, vec![timestamp_idx.into(), root_idx.into()])
                .await?;
            ensure!(
                proof.code_hash == CODE_HASH,
                "no or invalid beacon roots contract deployed; EIP-4788 is required"
            );
            let mut state = SingleContractState::from_proof(ADDRESS, proof)
                .context("invalid eth_getProof response")?;

            // validate the returned state and compute the return value
            let result = BeaconRootsContract::new(&mut state)?.get(timestamp);
            match result {
                Ok(returns) => Ok((returns, state)),
                Err(err) => match err {
                    Error::Reverted(_) => Err(anyhow!("BeaconRoots({timestamp}) reverted")),
                    err => Err(anyhow!(err)),
                },
            }
        }
    }
}

impl<'a> BeaconRootsContract<&'a mut SingleContractState> {
    /// Creates a new instance of the `BeaconRootsContract` from the given db.
    pub fn new(db: &'a mut SingleContractState) -> Result<Self, Error> {
        // retrieve the account data from the state trie using the contract's address hash
        let account = db.basic(ADDRESS)?.unwrap_or_default();
        // validate the account's code hash
        if account.code_hash != CODE_HASH {
            return Err(Error::InvalidContract);
        }

        Ok(Self { db })
    }

    /// Retrieves the root associated with the provided `calldata` (timestamp).
    ///
    /// This behaves exactly like the EVM bytecode defined in EIP-4788.
    pub fn get(&mut self, timestamp: U256) -> Result<B256, Error> {
        if timestamp.is_zero() {
            return Err(Error::Reverted("timestamp is zero"));
        }

        let timestamp_idx = timestamp % HISTORY_BUFFER_LENGTH;
        if self.db.storage(ADDRESS, timestamp_idx)? != timestamp {
            return Err(Error::Reverted("timestamp too old"));
        }

        let root_idx = timestamp_idx + HISTORY_BUFFER_LENGTH;
        let root = self.db.storage(ADDRESS, root_idx)?;

        Ok(root.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{host::db::ProviderDb, test_utils::get_el_url};
    use alloy::{
        network::BlockResponse,
        providers::{Provider, ProviderBuilder},
        rpc::types::BlockNumberOrTag as AlloyBlockNumberOrTag,
    };
    use test_log::test;

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn contract() {
        let el = ProviderBuilder::new().connect_http(get_el_url());

        // get the latest header
        let latest = el
            .get_block_by_number(AlloyBlockNumberOrTag::Latest)
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header = latest.header();
        let db = ProviderDb::new(el, Default::default(), header.hash);

        // query the contract for the latest timestamp, this should return parent_beacon_block_root
        let calldata = U256::from(header.timestamp);
        let (preflight, mut state) = BeaconRootsContract::preflight(db)
            .get(calldata)
            .await
            .expect("preflighting BeaconRootsContract failed");
        assert_eq!(state.root(), header.state_root);
        assert_eq!(preflight, header.parent_beacon_block_root.unwrap());

        // executing the contract from the exact state should return the same value
        assert_eq!(
            preflight,
            dbg!(BeaconRootsContract::new(&mut state).unwrap().get(calldata)).unwrap()
        );
    }
}
