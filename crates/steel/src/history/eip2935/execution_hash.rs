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

use crate::history::Error;
use alloy_primitives::{address, b256, Address, BlockNumber, B256, U256};
use anyhow::ensure;
use revm::Database;

/// Address where the EIP-2935 execution hash contract is deployed.
pub const ADDRESS: Address = address!("0x0000F90827F1C53a10cb7A02335B175320002935");

/// The length of the buffer that stores historical entries.
pub(crate) const HISTORY_SERVE_WINDOW: u64 = 8191;
/// Hash of the deployed EVM bytecode.
const CODE_HASH: B256 = b256!("0x6e49e66782037c0555897870e29fa5e552daf4719552131a0abce779daec0a5d");

/// Prepares a [SingleContractState] by retrieving the execution hash from an RPC provider and
/// constructing the necessary proofs.
///
/// It fetches the minimal set of Merkle proofs (for the contract's state and storage) required
/// to verify and retrieve the execution hash associated with the given `block_number`.
#[cfg(feature = "host")]
pub async fn preflight_get<N, P>(
    block_number: u64,
    provider: P,
    block_id: alloy::eips::BlockId,
) -> anyhow::Result<(B256, crate::history::state::SingleContractState)>
where
    N: alloy::network::Network,
    P: alloy::providers::Provider<N>,
{
    use crate::history::state::SingleContractState;
    use anyhow::{anyhow, Context};

    // compute the keys of the two storage slots that will be accessed
    let hash_idx = U256::from(block_number % HISTORY_SERVE_WINDOW);

    // derive the minimal state needed to query and validate
    let proof = provider
        .get_proof(ADDRESS, vec![hash_idx.into()])
        .block_id(block_id)
        .await
        .context("eth_getProof failed")?;
    ensure!(
        proof.code_hash == CODE_HASH,
        "no or invalid execution hash contract deployed; EIP-2935 is required"
    );
    let mut state =
        SingleContractState::from_proof(ADDRESS, proof).context("invalid eth_getProof response")?;

    // validate the returned state and compute the return value
    match ExecutionHashContract::get_from_db(&mut state, block_number) {
        Ok(returns) => Ok((returns, state)),
        Err(err) => match err {
            Error::Reverted => Err(anyhow!("ExecutionHashContract({}) reverted", block_number)),
            err => Err(err).context("RPC error"),
        },
    }
}

/// The `ExecutionHashContract` is responsible for storing and retrieving old execution hashes.
///
/// It is a reimplementation of the execution hash contract as defined in [EIP-2935](https://eips.ethereum.org/EIPS/eip-2935).
/// It is deployed at the address `0x0000F90827F1C53a10cb7A02335B175320002935` and has the
/// following storage layout:
/// - `hash_idx = block_number % HISTORY_BUFFER_LENGTH`: Stores the execution hash at this index.
pub struct ExecutionHashContract<D> {
    db: D,
}

impl<D> ExecutionHashContract<D>
where
    D: Database,
    Error: From<<D as Database>::Error>,
{
    /// Creates a new instance of the `ExecutionHashContract` from the given db.
    pub fn new(mut db: D) -> Result<Self, Error> {
        // retrieve the account data from the state trie using the contract's address hash
        let account = db.basic(ADDRESS)?.unwrap_or_default();
        // validate the account's code hash
        if account.code_hash != CODE_HASH {
            return Err(Error::NoContract);
        }

        Ok(Self { db })
    }

    /// Retrieves the execution hash associated with the provided `block_number`.
    ///
    /// This behaves exactly like the EVM bytecode defined in EIP-2935.
    pub fn get(&mut self, block_number: BlockNumber) -> Result<B256, Error> {
        let hash_idx = U256::from(block_number % HISTORY_SERVE_WINDOW);
        let hash = self.db.storage(ADDRESS, hash_idx)?;

        Ok(hash.into())
    }

    /// Retrieves the execution hash associated with the provided `block_number` from `db`.
    #[inline]
    pub fn get_from_db(db: D, block_number: BlockNumber) -> Result<B256, Error> {
        Self::new(db)?.get(block_number)
    }
}
