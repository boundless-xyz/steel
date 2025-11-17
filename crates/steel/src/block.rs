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

use crate::{
    BlockHeaderCommit, Commitment, CommitmentVersion, EvmBlockHeader, EvmEnv, EvmFactory,
    GuestEvmEnv, MerkleTrie, config::ChainSpec, state::StateDb,
};
use ::serde::{Deserialize, Serialize};
use alloy_consensus::TxReceipt;
use alloy_primitives::{B256, Bytes, Sealable, Sealed, map::HashMap};
use std::marker::PhantomData;

/// Input committing to the corresponding execution block hash.
#[derive(Clone, Serialize, Deserialize)]
pub struct BlockInput<F: EvmFactory> {
    header: F::Header,
    state_trie: MerkleTrie,
    storage_tries: Vec<MerkleTrie>,
    contracts: Vec<Bytes>,
    ancestors: Vec<F::Header>,
    receipts: Option<Vec<F::Receipt>>,
    #[serde(skip)]
    phantom: PhantomData<F>,
}

/// Implement [BlockHeaderCommit] for the unit type.
/// This makes it possible to treat an `HostEvmEnv<D, H, ()>`, which is used for the [BlockInput]
/// in the same way as any other `HostEvmEnv<D, H, BlockHeaderCommit>`.
impl<H: EvmBlockHeader> BlockHeaderCommit<H> for () {
    fn commit(self, header: &Sealed<H>, config_id: B256) -> Commitment {
        Commitment::new(
            CommitmentVersion::Block as u16,
            header.number(),
            header.seal(),
            config_id,
        )
    }
}

impl<F: EvmFactory> BlockInput<F> {
    /// Converts the input into a [EvmEnv] for verifiable state access in the guest.
    pub fn into_env(self, chain_spec: &ChainSpec<F::SpecId>) -> GuestEvmEnv<F> {
        // verify that the state root matches the state trie
        let state_root = self.state_trie.hash_slow();
        assert_eq!(self.header.state_root(), &state_root, "State root mismatch");

        // seal the header to compute its block hash
        let header = self.header.seal_slow();

        // validate that ancestor headers form a valid chain
        let mut block_hashes =
            HashMap::with_capacity_and_hasher(self.ancestors.len() + 1, Default::default());
        block_hashes.insert(header.number(), header.seal());

        let mut previous_header = header.inner();
        for ancestor in &self.ancestors {
            let ancestor_hash = ancestor.hash_slow();
            assert_eq!(
                previous_header.parent_hash(),
                &ancestor_hash,
                "Invalid ancestor chain: block {} is not the parent of block {}",
                ancestor.number(),
                previous_header.number()
            );
            block_hashes.insert(ancestor.number(), ancestor_hash);
            previous_header = ancestor;
        }

        // verify the root hash of the included receipts and extract their logs
        let logs = self.receipts.map(|receipts| {
            let root = alloy_trie::root::ordered_trie_root_with_encoder(&receipts, |r, out| {
                alloy_eips::eip2718::Encodable2718::encode_2718(r, out)
            });
            assert_eq!(header.receipts_root(), &root, "Receipts root mismatch");

            receipts
                .into_iter()
                .flat_map(TxReceipt::into_logs)
                .collect()
        });

        let db = StateDb::new(
            self.state_trie,
            self.storage_tries,
            self.contracts,
            block_hashes,
            logs,
        );
        let commit = Commitment::new(
            CommitmentVersion::Block as u16,
            header.number(),
            header.seal(),
            chain_spec.digest(),
        );

        EvmEnv::from_chain_spec(db, chain_spec, header, commit)
    }
}

#[cfg(feature = "host")]
pub mod host {
    use super::BlockInput;
    use crate::{
        EvmBlockHeader, EvmFactory,
        host::db::{ProofDb, ProviderDb},
    };
    use alloy::{network::Network, providers::Provider};
    use alloy_primitives::Sealed;
    use anyhow::{anyhow, ensure};
    use log::debug;
    use std::{fmt::Display, marker::PhantomData};

    impl<F: EvmFactory> BlockInput<F> {
        /// Creates the `BlockInput` containing the necessary EVM state that can be verified against
        /// the block hash.
        pub(crate) async fn from_proof_db<N, P>(
            mut db: ProofDb<ProviderDb<N, P>>,
            header: Sealed<F::Header>,
        ) -> anyhow::Result<Self>
        where
            N: Network,
            P: Provider<N>,
            F::Header: TryFrom<<N as Network>::HeaderResponse>,
            <F::Header as TryFrom<<N as Network>::HeaderResponse>>::Error: Display,
            F::Receipt: TryFrom<<N as Network>::ReceiptResponse>,
            <F::Receipt as TryFrom<<N as Network>::ReceiptResponse>>::Error: Display,
        {
            assert_eq!(db.inner().block(), header.seal(), "DB block mismatch");

            let (state_trie, storage_tries) = db.state_proof().await?;
            ensure!(
                header.state_root() == &state_trie.hash_slow(),
                "accountProof root does not match header's stateRoot"
            );

            // collect the bytecode of all referenced contracts
            let contracts: Vec<_> = db.contracts().values().cloned().collect();

            // retrieve ancestor block headers
            let ancestor_proof = db.ancestor_proof(header.number()).await?;
            let ancestors = ancestor_proof
                .into_iter()
                .map(F::Header::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| anyhow!("invalid header: {err}"))?;

            // retrieve receipt envelops
            let receipt_proof = db.receipt_proof().await?;
            let receipts = receipt_proof
                .map(|receipts| receipts.into_iter().map(F::Receipt::try_from).collect())
                .transpose()
                .map_err(|err| anyhow!("invalid receipt: {err}"))?;

            debug!("state size: {}", state_trie.size());
            debug!("storage tries: {}", storage_tries.len());
            debug!(
                "total storage size: {}",
                storage_tries.iter().map(|t| t.size()).sum::<usize>()
            );
            debug!("contracts: {}", contracts.len());
            debug!("ancestor blocks: {}", ancestors.len());
            debug!("receipts: {:?}", receipts.as_ref().map(Vec::len));

            let input = BlockInput {
                header: header.into_inner(),
                state_trie,
                storage_tries,
                contracts,
                ancestors,
                receipts,
                phantom: PhantomData,
            };

            Ok(input)
        }
    }
}
