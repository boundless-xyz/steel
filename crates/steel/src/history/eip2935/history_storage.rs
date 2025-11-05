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

use crate::history::{Error, SingleContractState};
use alloy_primitives::{Address, B256, U256, address, b256, uint};
use revm::Database;

/// Address where the EIP-2935 execution hash contract is deployed.
pub const ADDRESS: Address = address!("0x0000F90827F1C53a10cb7A02335B175320002935");
/// The length of the buffer that stores historical entries.
pub const HISTORY_SERVE_WINDOW: U256 = uint!(8191_U256);

/// Hash of the deployed EVM bytecode.
const CODE_HASH: B256 = b256!("0x6e49e66782037c0555897870e29fa5e552daf4719552131a0abce779daec0a5d");

/// The `HistoryStorageContract` is responsible for storing and retrieving old execution hashes.
///
/// It is a reimplementation of the execution hash contract as defined in [EIP-2935](https://eips.ethereum.org/EIPS/eip-2935).
/// It is deployed at the address `0x0000F90827F1C53a10cb7A02335B175320002935` and has the
/// following storage layout:
/// - `hash_idx = block_number % HISTORY_BUFFER_LENGTH`: Stores the execution hash at this index.
pub struct HistoryStorageContract<D> {
    db: D,
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use crate::{history::SingleContractState, host::db::ProviderDb};
    use alloy::providers::{Network, Provider};
    use anyhow::{Context, anyhow, ensure};

    impl<N, P> HistoryStorageContract<ProviderDb<N, P>>
    where
        N: Network,
        P: Provider<N>,
    {
        /// Creates a new instance of the `ExecutionHashContract` from the given db.
        pub fn preflight(db: ProviderDb<N, P>) -> Self {
            Self { db }
        }

        pub async fn get_unchecked(
            &self,
            block_number: U256,
        ) -> anyhow::Result<(B256, SingleContractState)> {
            // compute the keys of the two storage slots that will be accessed
            let hash_idx = block_number % HISTORY_SERVE_WINDOW;

            // derive the minimal state needed to query and validate
            let proof = self.db.get_proof(ADDRESS, vec![hash_idx.into()]).await?;
            ensure!(
                proof.code_hash == CODE_HASH,
                "no or invalid history storage contract deployed; EIP-2935 is required"
            );
            let mut state = SingleContractState::from_proof(ADDRESS, proof)
                .context("invalid eth_getProof response")?;

            // validate the returned state and compute the return value
            let result = HistoryStorageContract::new(&mut state)?.get_unchecked(block_number);
            match result {
                Ok(returns) => Ok((returns, state)),
                Err(err) => Err(anyhow!(err)),
            }
        }
    }
}

impl<'a> HistoryStorageContract<&'a mut SingleContractState> {
    /// Creates a new instance of the `HistoryStorageContract` from the given db.
    pub fn new(db: &'a mut SingleContractState) -> Result<Self, Error> {
        // retrieve the account data from the state trie using the contract's address hash
        let account = db.basic(ADDRESS)?.unwrap_or_default();
        // validate the account's code hash
        if account.code_hash != CODE_HASH {
            return Err(Error::InvalidContract);
        }

        Ok(Self { db })
    }

    /// Retrieves the execution hash associated with the provided `block_number`.
    ///
    /// In contrast to the EVM bytecode defined in EIP-2935, this does not check whether the request
    /// is indeed in the range of [block.number-`HISTORY_SERVE_WINDOW`, block.number-1].
    pub fn get_unchecked(&mut self, block_number: U256) -> Result<B256, Error> {
        let hash_idx = block_number % HISTORY_SERVE_WINDOW;
        let hash = self.db.storage(ADDRESS, hash_idx)?;

        Ok(hash.into())
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
        // TODO: Make this an Anvil provider, once Anvil has EIP-2935 support
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
        let block_number = U256::from(header.number - 1);
        let (preflight, mut state) = HistoryStorageContract::preflight(db)
            .get_unchecked(block_number)
            .await
            .expect("preflighting HistoryStorageContract failed");
        assert_eq!(state.root(), header.state_root);
        assert_eq!(preflight, header.parent_hash);

        // executing the contract from the exact state should return the same value
        assert_eq!(
            preflight,
            dbg!(
                HistoryStorageContract::new(&mut state)
                    .unwrap()
                    .get_unchecked(block_number)
            )
            .unwrap()
        );
    }
}
