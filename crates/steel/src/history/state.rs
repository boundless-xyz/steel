use crate::{MerkleTrie, StateAccount};
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rpc_types::EIP1186AccountProofResponse;
use revm::{bytecode::Bytecode, context::DBErrorMarker, state::AccountInfo, Database};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("wrong address")]
    AddressMismatch,
    /// Error indicating that the contract is not deployed at the expected address.
    #[error("wrong or no contract deployed")]
    NoContract,
    /// Error indicating an inconsistency in the contract's state.
    #[error("inconsistent state")]
    InvalidState,
    /// Error indicating that the state contains improperly encoded data.
    #[error("state contains invalid encoded data")]
    InvalidEncoding(#[from] alloy_rlp::Error),
    /// Error indicating that the contract execution was reverted.
    #[error("execution reverted")]
    Reverted,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct SingleContractState {
    address: Address,
    state_trie: MerkleTrie,
    storage_trie: MerkleTrie,
}

impl SingleContractState {
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

    /// Computes the state root.
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
            return Err(Error::AddressMismatch);
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
            return Err(Error::AddressMismatch);
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
