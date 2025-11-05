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

use crate::{MerkleTrie, StateAccount};
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_rpc_types::EIP1186AccountProofResponse;
use revm::{Database, bytecode::Bytecode, context::DBErrorMarker, state::AccountInfo};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

/// Possible errors that can occur within a [SingleContractState] contract execution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Error indicating that the contract is not deployed at the expected address.
    #[error("wrong or no contract deployed")]
    InvalidContract,
    /// The state for a different address was accessed.
    #[error("accessed an invalid address for this state")]
    InvalidAddress,
    /// Error indicating an inconsistency in the contract's state.
    #[error("inconsistent state")]
    InvalidState,
    /// Error indicating that the state contains improperly encoded data.
    #[error("state contains invalid encoded data")]
    InvalidEncoding(#[from] alloy_rlp::Error),
    /// Error indicating that the contract execution was reverted.
    #[error("execution reverted: {0}")]
    Reverted(&'static str),
    /// Unspecified error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl DBErrorMarker for Error {}

impl From<Infallible> for Error {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}

#[cfg(feature = "host")]
impl From<crate::host::db::provider::Error> for Error {
    fn from(value: crate::host::db::provider::Error) -> Self {
        anyhow::Error::new(value).into()
    }
}

/// Simplified read-only EVM database that encapsulates the state of a single smart contract.
///
/// This struct is created from an EIP-1186 proof response (`eth_getProof`) and holds the minimal
/// state required for `revm` to execute calls against the specified contract. It contains the
/// Merkle Tries for the contract's account state and its storage, ensuring that any data
/// accessed through its `Database` implementation is verifiable against its state root.
#[derive(Clone, Serialize, Deserialize)]
pub struct SingleContractState {
    address: Address,
    state_trie: MerkleTrie,
    storage_trie: MerkleTrie,
}

impl SingleContractState {
    /// Creates a new `SingleContractState` instance from an `eth_getProof` RPC response.
    #[allow(dead_code)]
    pub fn from_proof(address: Address, proof: EIP1186AccountProofResponse) -> Result<Self, Error> {
        Ok(Self {
            address,
            state_trie: MerkleTrie::from_rlp_nodes(proof.account_proof)?,
            storage_trie: MerkleTrie::from_rlp_nodes(
                proof.storage_proof.iter().flat_map(|p| &p.proof),
            )?,
        })
    }

    /// Computes and returns the state root of the encapsulated world state.
    ///
    /// The root is the hash of the `state_trie`. This value can be compared against a block
    /// header's `stateRoot` to verify the integrity of the state contained within this instance.
    #[inline]
    pub fn root(&self) -> B256 {
        self.state_trie.hash_slow()
    }
}

/// Implements the Database trait, but only for the account of the beacon roots contract.
impl Database for SingleContractState {
    type Error = Error;

    #[inline]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // only allow accessing the beacon roots contract's address
        if address != self.address {
            return Err(Error::InvalidAddress);
        }

        let account: StateAccount = self
            .state_trie
            .get_rlp(keccak256(self.address))?
            .unwrap_or_default();
        // and the account storage must match the storage trie
        if account.storage_root != self.storage_trie.hash_slow() {
            return Err(Error::InvalidState);
        }

        Ok(Some(AccountInfo {
            balance: account.balance,
            nonce: account.nonce,
            code_hash: account.code_hash,
            code: None,
        }))
    }

    fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        unimplemented!("code_by_hash should not be called")
    }

    #[inline]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // only allow accessing the beacon roots contract's address
        if address != self.address {
            return Err(Error::InvalidAddress);
        }

        Ok(self
            .storage_trie
            .get_rlp(keccak256(index.to_be_bytes::<32>()))?
            .unwrap_or_default())
    }

    fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
        unimplemented!("block_hash should not be called")
    }
}
