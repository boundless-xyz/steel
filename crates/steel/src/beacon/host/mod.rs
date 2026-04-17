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

use super::*;
use crate::{EvmBlockHeader, ethereum::EthBlockHeader};
use alloy::{network::Ethereum, providers::Provider};
use alloy_primitives::B256;
use anyhow::{Context, bail, ensure};
use client::BeaconClient;
use lighthouse_types::{BeaconBlockRef, ForkName, FullPayloadRef, MainnetEthSpec};
use sha2::{Digest, Sha256};
use tree_hash::TreeHash;

pub(crate) mod client;

impl BeaconCommit {
    /// Creates a new `BeaconCommit` for the provided header which proofs the inclusion of the
    /// corresponding block hash in the referenced beacon block.
    pub(crate) async fn from_header<P>(
        header: &Sealed<EthBlockHeader>,
        commitment_version: CommitmentVersion,
        rpc_provider: P,
        beacon_client: &BeaconClient,
    ) -> anyhow::Result<Self>
    where
        P: Provider<Ethereum>,
    {
        let (commit, beacon_root) =
            create_beacon_commit(header, commitment_version, rpc_provider, beacon_client).await?;

        log::debug!(
            "Committing to beacon block: {{ {}, root: {} }}",
            commit.block_id(),
            beacon_root,
        );

        Ok(commit)
    }
}

impl<const LEAF_INDEX: usize> GeneralizedBeaconCommit<LEAF_INDEX> {
    /// Builds a `GeneralizedBeaconCommit` proving the value at `LEAF_INDEX` inside the beacon
    /// block identified by `parent_beacon_root`.
    pub(crate) async fn from_beacon_root(
        parent_beacon_root: B256,
        beacon_client: &BeaconClient,
        block_id: BeaconBlockId,
    ) -> anyhow::Result<Self> {
        let signed_beacon_block = beacon_client
            .get_block(parent_beacon_root)
            .await
            .with_context(|| format!("failed to get block {parent_beacon_root}"))?;
        let fork = signed_beacon_block.fork_name_unchecked();
        ensure!(
            fork >= ForkName::Deneb,
            "invalid version of block {parent_beacon_root}: expected >= {}, got {fork}",
            ForkName::Deneb,
        );
        let block = signed_beacon_block.message();
        let full_ep = block
            .execution_payload()
            .map_err(|e| anyhow::anyhow!("block has no execution payload: {e:?}"))?;
        let ep = full_ep.execution_payload_ref();

        let expected_leaf = match LEAF_INDEX {
            BLOCK_HASH_LEAF_INDEX => ep.block_hash().tree_hash_root(),
            STATE_ROOT_LEAF_INDEX => ep.state_root().tree_hash_root(),
            _ => bail!("unsupported LEAF_INDEX {LEAF_INDEX}"),
        };

        let proof = prove_execution_payload_field(&block, full_ep, LEAF_INDEX)?;
        let commit = GeneralizedBeaconCommit::new(proof, block_id);
        commit
            .verify(expected_leaf, parent_beacon_root)
            .with_context(|| {
                format!(
                    "LEAF_INDEX {LEAF_INDEX} does not point to the expected ExecutionPayload field"
                )
            })?;

        Ok(commit)
    }
}

/// Creates a beacon commitment that the field at `LEAF_INDEX` is contained in the
/// `ExecutionPayload` of the beacon block corresponding to `header` creating a
/// [CommitmentVersion::Beacon] commitment.
async fn create_eip4788_beacon_commit<P, H, const LEAF_INDEX: usize>(
    header: &Sealed<H>,
    rpc_provider: P,
    beacon_client: &BeaconClient,
) -> anyhow::Result<(GeneralizedBeaconCommit<LEAF_INDEX>, B256)>
where
    P: Provider<Ethereum>,
    H: EvmBlockHeader,
{
    let child = {
        let child_number = header.number() + 1;
        let block_res = rpc_provider
            .get_block_by_number(child_number.into())
            .await
            .context("eth_getBlockByNumber failed")?;
        let block = block_res.with_context(|| {
            format!(
                "beacon block commitment cannot be created for the most recent block; \
                    use `parent` tag instead: block {} does not have a child",
                header.number()
            )
        })?;
        block.header
    };
    ensure!(
        child.parent_hash == header.seal(),
        "API returned invalid child block"
    );

    let beacon_root = child
        .parent_beacon_block_root
        .context("parent_beacon_block_root missing in execution header")?;
    let commit = GeneralizedBeaconCommit::from_beacon_root(
        beacon_root,
        beacon_client,
        BeaconBlockId::Eip4788(child.timestamp),
    )
    .await?;

    Ok((commit, beacon_root))
}

/// Creates a beacon commitment that the field at `LEAF_INDEX` is contained in the
/// `ExecutionPayload` of the beacon block corresponding to `header` creating a
/// [CommitmentVersion::Consensus] commitment.
async fn create_slot_beacon_commit<P, H, const LEAF_INDEX: usize>(
    header: &Sealed<H>,
    rpc_provider: P,
    beacon_client: &BeaconClient,
) -> anyhow::Result<(GeneralizedBeaconCommit<LEAF_INDEX>, B256)>
where
    P: Provider<Ethereum>,
    H: EvmBlockHeader,
{
    // query the beacon block corresponding to the given execution header
    let (beacon_root, beacon_header) = {
        // first, retrieve the corresponding full execution header
        let execution_header = rpc_provider
            .get_block_by_hash(header.seal())
            .await
            .context("eth_getBlockByHash failed")?
            .with_context(|| format!("block {} not found", header.seal()))?
            .header;
        let parent_root = execution_header
            .parent_beacon_block_root
            .context("parent_beacon_block_root missing in execution header")?;
        // then, retrieve the beacon header that contains the same parent root
        let response = beacon_client
            .get_header_for_parent_root(parent_root)
            .await
            .with_context(|| format!("failed to get header for parent root {parent_root}"))?;
        ensure!(
            response.header.message.parent_root == parent_root,
            "API returned invalid beacon header"
        );
        (response.root, response.header.message)
    };
    let commit = GeneralizedBeaconCommit::from_beacon_root(
        beacon_root,
        beacon_client,
        BeaconBlockId::Slot(beacon_header.slot.as_u64()),
    )
    .await?;

    Ok((commit, beacon_root))
}

/// Creates a beacon commitment that the field at `LEAF_INDEX` is contained in the
/// `ExecutionPayload` of the beacon block corresponding to `header`.
pub(crate) async fn create_beacon_commit<P, H, const LEAF_INDEX: usize>(
    header: &Sealed<H>,
    commitment_version: CommitmentVersion,
    rpc_provider: P,
    beacon_client: &BeaconClient,
) -> anyhow::Result<(GeneralizedBeaconCommit<LEAF_INDEX>, B256)>
where
    P: Provider<Ethereum>,
    H: EvmBlockHeader,
{
    match commitment_version {
        CommitmentVersion::Beacon => {
            create_eip4788_beacon_commit(header, rpc_provider, beacon_client).await
        }
        CommitmentVersion::Consensus => {
            create_slot_beacon_commit(header, rpc_provider, beacon_client).await
        }
        _ => bail!("invalid commitment version"),
    }
}

/// A binary Merkle tree stored as a flat array indexed by [generalized index].
///
/// Nodes are addressed by generalized index: position 1 is the root, and the children of
/// node `k` are `2k` and `2k + 1`. Position 0 is unused.
///
/// [generalized index]: https://github.com/ethereum/consensus-specs/blob/master/ssz/merkle-proofs.md
struct MerkleTree(Vec<B256>);

impl MerkleTree {
    /// Builds a Merkle tree from `leaves`, padding to the next power of two with zero hashes.
    fn new(leaves: &[B256]) -> Self {
        let num_leaves = leaves.len().next_power_of_two();
        let mut tree = vec![B256::ZERO; 2 * num_leaves];
        tree[num_leaves..num_leaves + leaves.len()].copy_from_slice(leaves);

        let mut hasher = Sha256::new();
        for i in (1..num_leaves).rev() {
            hasher.update(tree[2 * i]);
            hasher.update(tree[2 * i + 1]);
            tree[i].copy_from_slice(&hasher.finalize_reset());
        }
        MerkleTree(tree)
    }

    /// Returns the Merkle root.
    fn root(&self) -> B256 {
        self.0[1]
    }

    /// Returns the number of leaves (padded to the next power of two).
    fn num_leaves(&self) -> usize {
        self.0.len() / 2
    }

    /// Computes the Merkle proof for the node at `gindex`.
    ///
    /// Returns the sibling hashes along the path from the node to the root, equivalent to
    /// looking up [`get_branch_indices`] in the tree.
    ///
    /// [`get_branch_indices`]: https://github.com/ethereum/consensus-specs/blob/master/ssz/merkle-proofs.md
    fn proof(&self, leaf_index: usize) -> Vec<B256> {
        assert!(leaf_index < self.num_leaves());
        let mut gindex = self.num_leaves() + leaf_index;

        let mut branch = Vec::with_capacity(gindex.ilog2() as usize);
        while gindex > 1 {
            branch.push(self.0[gindex ^ 1]); // sibling
            gindex >>= 1; // parent
        }
        branch
    }
}

/// Collects `tree_hash_root()` of each `ExecutionPayload` field, in SSZ container order.
///
/// Must only be called for blocks >= Deneb (enforced by the caller).
fn execution_payload_leaves(ep: FullPayloadRef<'_, MainnetEthSpec>) -> Vec<B256> {
    let ep = ep.execution_payload_ref();
    vec![
        ep.parent_hash().tree_hash_root(),
        ep.fee_recipient().tree_hash_root(),
        ep.state_root().tree_hash_root(),
        ep.receipts_root().tree_hash_root(),
        ep.logs_bloom().tree_hash_root(),
        ep.prev_randao().tree_hash_root(),
        ep.block_number().tree_hash_root(),
        ep.gas_limit().tree_hash_root(),
        ep.gas_used().tree_hash_root(),
        ep.timestamp().tree_hash_root(),
        ep.extra_data().tree_hash_root(),
        ep.base_fee_per_gas().tree_hash_root(),
        ep.block_hash().tree_hash_root(),
        ep.transactions().tree_hash_root(),
        ep.withdrawals()
            .expect("Deneb+ blocks have withdrawals")
            .tree_hash_root(),
        ep.blob_gas_used()
            .expect("Deneb+ blocks have blob_gas_used")
            .tree_hash_root(),
        ep.excess_blob_gas()
            .expect("Deneb+ blocks have excess_blob_gas")
            .tree_hash_root(),
    ]
}

/// Collects `tree_hash_root()` of each `BeaconBlock` field, in SSZ container order.
fn block_leaves(block: &BeaconBlockRef<'_, MainnetEthSpec>) -> Vec<B256> {
    vec![
        block.slot().tree_hash_root(),
        block.proposer_index().tree_hash_root(),
        block.parent_root().tree_hash_root(),
        block.state_root().tree_hash_root(),
        block.body_root(),
    ]
}

/// Generates the full 3-level Merkle inclusion proof for a field within the `ExecutionPayload`
/// of the given `BeaconBlock`, identified by its `gindex` (generalized index) in the full
/// beacon block tree.
fn prove_execution_payload_field(
    block: &BeaconBlockRef<'_, MainnetEthSpec>,
    ep: FullPayloadRef<'_, MainnetEthSpec>,
    gindex: usize,
) -> anyhow::Result<Vec<B256>> {
    const BODY_INDEX_IN_BLOCK: usize = 4;
    const EP_INDEX_IN_BODY: usize = 9;

    let ep_tree = MerkleTree::new(&execution_payload_leaves(ep));
    let body_tree = MerkleTree::new(&block.body().body_merkle_leaves());
    let block_tree = MerkleTree::new(&block_leaves(block));

    // guard against fork-driven field reorderings producing invalid proofs
    ensure!(
        ep_tree.root() == ep.tree_hash_root(),
        "ExecutionPayload root mismatch: execution_payload_leaves() is out of date"
    );
    ensure!(
        block_tree.root() == block.tree_hash_root(),
        "BeaconBlock root mismatch: block_leaves() is out of date"
    );

    // a valid EP-field gindex must decompose to (1, BODY_INDEX_IN_BLOCK, EP_INDEX_IN_BODY, _)
    let leaf_index = gindex % ep_tree.num_leaves();
    let g = gindex / ep_tree.num_leaves();
    let body_index = g % body_tree.num_leaves();
    let g = g / body_tree.num_leaves();
    let block_index = g % block_tree.num_leaves();
    let g = g / block_tree.num_leaves();
    ensure!(
        g == 1 && block_index == BODY_INDEX_IN_BLOCK && body_index == EP_INDEX_IN_BODY,
        "gindex {gindex} does not point to a field inside the ExecutionPayload"
    );

    let mut proof = Vec::with_capacity(gindex.ilog2() as usize);
    proof.extend(ep_tree.proof(leaf_index));
    proof.extend(body_tree.proof(body_index));
    proof.extend(block_tree.proof(block_index));

    Ok(proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{get_cl_url, get_el_url};
    use alloy::{eips::BlockNumberOrTag, network::BlockResponse, providers::ProviderBuilder};

    #[tokio::test]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn create_eip4788_beacon_commit() {
        let el = ProviderBuilder::new().connect_http(get_el_url());
        let cl = BeaconClient::new(get_cl_url()).unwrap();

        let block = el
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();

        let timestamp = block.header().timestamp;
        let parent_beacon_root = block.header().parent_beacon_block_root.unwrap();

        let block = el
            .get_block_by_hash(block.header().parent_hash)
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header: Sealed<EthBlockHeader> = Sealed::new(block.header.try_into().unwrap());

        let (commit, _): (BeaconCommit, B256) =
            super::create_eip4788_beacon_commit(&header, &el, &cl)
                .await
                .unwrap();

        // verify the commitment by querying the beacon client
        let (block_id, block_root) = dbg!(commit.into_commit(header.seal()));
        assert_eq!(block_id.as_id(), timestamp);
        assert_eq!(block_root, parent_beacon_root);
    }

    #[tokio::test]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn create_slot_beacon_commit() {
        let el = ProviderBuilder::new().connect_http(get_el_url());
        let cl = BeaconClient::new(get_cl_url()).unwrap();

        let block = el
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header: Sealed<EthBlockHeader> = Sealed::new(block.header.try_into().unwrap());

        let (commit, _): (BeaconCommit, B256) = super::create_slot_beacon_commit(&header, &el, &cl)
            .await
            .unwrap();

        // verify the commitment by querying the beacon client
        let (block_id, block_root) = dbg!(commit.into_commit(header.seal()));
        let beacon_block = cl.get_block(block_id.as_id()).await.unwrap();
        assert_eq!(block_root, beacon_block.message().tree_hash_root());
    }
}
