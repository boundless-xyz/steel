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
    Commitment, CommitmentVersion, EvmBlockHeader, EvmFactory, EvmSpecId, GuestEvmEnv,
    precompiles::{BeaconRootsContract, HistoryStorageContract},
};
use alloy_primitives::{B256, BlockNumber, U256};
use anyhow::{bail, ensure};

/// Number of block hashes the verifier can access via the BLOCKHASH opcode.
pub const HISTORY_LIMIT: u64 = revm::primitives::BLOCK_HASH_HISTORY;
/// Number of block hashes the verifier can access via the EIP2935 history storage contract.
pub const EIP2935_HISTORY_LIMIT: u64 = alloy_eips::eip2935::HISTORY_SERVE_WINDOW as u64;

/// Represents a verifier for validating Steel commitments within Steel.
///
/// The verifier is used to validate Steel commitments representing a historical blockchain state.
///
/// ### Usage
/// - **Preflight verification on the Host:** To prepare verification on the host environment and
///   build the necessary proof, use [SteelVerifier::preflight]. The environment can be initialized
///   using the [EthEvmEnv::builder] or [EvmEnv::builder].
/// - **Verification in the Guest:** To initialize the verifier in the guest environment, use
///   [SteelVerifier::new]. The environment should be constructed using [EvmInput::into_env].
///
/// ### Examples
/// ```rust,no_run
/// # use risc0_steel::{ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv}, SteelVerifier, Commitment};
/// # use url::Url;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> anyhow::Result<()> {
/// // Host:
/// let rpc_url = Url::parse("https://ethereum-rpc.publicnode.com")?;
/// let mut env = EthEvmEnv::builder().rpc(rpc_url).chain_spec(&ETH_MAINNET_CHAIN_SPEC).build().await?;
///
/// // Preflight the verification of a commitment
/// let commitment = Commitment::default(); // Your commitment here
/// SteelVerifier::preflight(&mut env).verify(&commitment).await?;
///
/// let evm_input = env.into_input().await?;
///
/// // Guest:
/// let evm_env = evm_input.into_env(&ETH_MAINNET_CHAIN_SPEC);
/// let verifier = SteelVerifier::new(&evm_env);
/// verifier.verify(&commitment); // Panics if verification fails
/// # Ok(())
/// # }
/// ```
///
/// [EthEvmEnv::builder]: crate::ethereum::EthEvmEnv
/// [EvmEnv::builder]: crate::EvmEnv
/// [EvmInput::into_env]: crate::EvmInput::into_env
pub struct SteelVerifier<E> {
    env: E,
}

impl<'a, F: EvmFactory> SteelVerifier<&'a GuestEvmEnv<F>> {
    /// Constructor for verifying Steel commitments in the guest.
    pub fn new(env: &'a GuestEvmEnv<F>) -> Self {
        Self { env }
    }

    /// Verifies the commitment in the guest and panics on failure.
    ///
    /// This includes checking that the `commitment.configID` matches the
    /// configuration ID associated with the current guest environment (`self.env.commit.configID`).
    #[inline]
    pub fn verify(&self, commitment: &Commitment) {
        self.verify_with_config_id(commitment, self.env.commit.configID);
    }

    /// Verifies the commitment in the guest against an explicitly provided configuration ID,
    /// and panics on failure.
    pub fn verify_with_config_id(&self, commitment: &Commitment, config_id: B256) {
        assert_eq!(commitment.configID, config_id, "Invalid config ID");
        let (id, version_code) = commitment.decode_id();
        match CommitmentVersion::n(version_code) {
            Some(CommitmentVersion::Block) => {
                let header = self.env.header().inner();
                let block_number = validate_block_number(id, header).expect("Invalid block number");
                // use the header field for a direct parent
                let block_hash = if block_number + 1 == header.number() {
                    *header.parent_hash()
                }
                // use the history storage contract when it contains the block
                else if let Some(hash) = self.history_storage_hash(id) {
                    hash
                }
                // otherwise emulate the BLOCKHASH opcode
                else {
                    validate_blockhash_window(header, block_number).expect("Invalid block number");
                    self.env.db().block_hash(block_number)
                };
                assert_eq!(block_hash, commitment.digest, "Invalid block hash");
            }
            Some(CommitmentVersion::Beacon) => {
                assert!(self.env.spec_id.has_eip4788(), "EIP-4788 required");
                // beacon roots contract reverts when `id` id not in allowed history window
                let beacon_root = BeaconRootsContract::new(self.env).call(id);
                // an unset root slot reads as zero and must never validate a commitment
                assert!(!beacon_root.is_zero(), "Invalid beacon root");
                assert_eq!(beacon_root, commitment.digest, "Invalid beacon root");
            }
            _ => {
                unimplemented!(
                    "Unsupported commitment version: {}",
                    Commitment::version_name(version_code)
                )
            }
        }
    }

    /// Returns the block hash for `id` from the EIP-2935 history storage contract, or `None`
    /// when the contract cannot serve it.
    fn history_storage_hash(&self, id: U256) -> Option<B256> {
        if !self.env.spec_id.has_eip2935() {
            return None;
        }
        // the contract reverts when `id` is not in the allowed history window
        let hash = HistoryStorageContract::new(self.env).call(id);
        // an unset slot reads as zero, i.e. the block pre-dates the fork activation
        (!hash.is_zero()).then_some(hash)
    }
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use crate::host::{HostEvmEnv, db::ProviderDb};
    use alloy::providers::{Network, Provider};
    use anyhow::Context;
    use revm::Database;

    impl<'a, F, N, P, C> SteelVerifier<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        F: EvmFactory,
        N: Network,
        P: Provider<N> + 'static,
    {
        /// Constructor for preflighting Steel commitment verifications on the host.
        ///
        /// Initializes the environment for verifying Steel commitments, fetching necessary data via
        /// RPC, and generating a storage proof for any accessed elements using
        /// [EvmEnv::into_input].
        ///
        /// [EvmEnv::into_input]: crate::EvmEnv::into_input
        pub fn preflight(env: &'a mut HostEvmEnv<ProviderDb<N, P>, F, C>) -> Self {
            Self { env }
        }

        /// Preflights the commitment verification on the host.
        ///
        /// This includes checking that the `commitment.configID` matches the
        /// configuration ID associated with the current host environment.
        #[inline]
        pub async fn verify(self, commitment: &Commitment) -> anyhow::Result<()> {
            let config_id = self.env.commit.config_id();
            self.verify_with_config_id(commitment, config_id).await
        }

        /// Preflights the commitment verification on the host against an explicitly provided
        /// configuration ID.
        pub async fn verify_with_config_id(
            mut self,
            commitment: &Commitment,
            config_id: B256,
        ) -> anyhow::Result<()> {
            log::debug!("Executing preflight verifying {commitment:?}");

            ensure!(commitment.configID == config_id, "invalid config ID");
            let (id, version_code) = commitment.decode_id();
            match CommitmentVersion::n(version_code) {
                Some(CommitmentVersion::Block) => {
                    let block_number = validate_block_number(id, self.env.header().inner())
                        .context("invalid block number")?;
                    // use the header field for a direct parent
                    let block_hash = if block_number + 1 == self.env.header().inner().number() {
                        *self.env.header().inner().parent_hash()
                    }
                    // use the history storage contract when it contains the block
                    else if let Some(hash) = self.preflight_history_storage_hash(id).await? {
                        hash
                    }
                    // otherwise emulate the BLOCKHASH opcode
                    else {
                        validate_blockhash_window(self.env.header().inner(), block_number)
                            .context("invalid block number")?;
                        self.env
                            .spawn_with_db(move |db| db.block_hash(block_number))
                            .await?
                    };
                    ensure!(block_hash == commitment.digest, "invalid block hash");

                    Ok(())
                }
                Some(CommitmentVersion::Beacon) => {
                    ensure!(self.env.spec_id.has_eip4788(), "EIP-4788 required");
                    let beacon_root = BeaconRootsContract::preflight(self.env).call(id).await?;
                    // an unset root slot reads as zero and must never validate a commitment
                    ensure!(!beacon_root.is_zero(), "invalid beacon root");
                    ensure!(beacon_root == commitment.digest, "invalid beacon root");

                    Ok(())
                }
                _ => unimplemented!(
                    "Unsupported commitment version: {}",
                    Commitment::version_name(version_code)
                ),
            }
        }

        /// Preflights the lookup of the block hash for `id` in the EIP-2935 history storage
        /// contract, returning `None` when the contract cannot serve it.
        async fn preflight_history_storage_hash(
            &mut self,
            id: U256,
        ) -> anyhow::Result<Option<B256>> {
            if !self.env.spec_id.has_eip2935() {
                return Ok(None);
            }
            // the contract reverts when `id` is not in the allowed history window
            let hash = HistoryStorageContract::preflight(self.env)
                .call(id)
                .await
                .with_context(|| {
                    format!("only valid for the {EIP2935_HISTORY_LIMIT} most recent blocks")
                })?;
            // an unset slot reads as zero, i.e. the block pre-dates the fork activation
            Ok((!hash.is_zero()).then_some(hash))
        }
    }
}

fn validate_block_number(n: U256, header: &impl EvmBlockHeader) -> anyhow::Result<u64> {
    match BlockNumber::try_from(n) {
        Ok(n) if n < header.number() => Ok(n),
        _ => bail!("not an ancestor"),
    }
}

fn validate_blockhash_window(
    header: &impl EvmBlockHeader,
    number: BlockNumber,
) -> anyhow::Result<()> {
    ensure!(
        number < header.number() && header.number() - number <= HISTORY_LIMIT,
        "only valid for the {HISTORY_LIMIT} most recent blocks"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CommitmentVersion,
        config::ChainSpec,
        ethereum::{ETH_MAINNET_CHAIN_SPEC, EthEvmEnv},
        test_utils::get_el_url,
    };
    use alloy::{
        consensus::BlockHeader,
        network::{BlockResponse, TransactionBuilder, primitives::HeaderResponse},
        providers::{Provider, ProviderBuilder, ext::AnvilApi},
        rpc::types::{BlockNumberOrTag as AlloyBlockNumberOrTag, TransactionRequest},
    };
    use alloy_eips::eip2935::HISTORY_STORAGE_ADDRESS;
    use alloy_primitives::{Address, Bytes, address, bytes};
    use revm::primitives::hardfork::SpecId;
    use test_log::test;

    /// Creates a block commitment to the block `n` blocks below the current head.
    async fn block_commitment(
        el: &impl Provider,
        chain_spec: &ChainSpec<SpecId>,
        n: u64,
    ) -> Commitment {
        let latest = el.get_block_number().await.unwrap();
        let block = el
            .get_block_by_number((latest - n).into())
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header = block.header();
        Commitment::new(
            CommitmentVersion::Block as u16,
            header.number(),
            header.hash(),
            chain_spec.digest(),
        )
    }

    async fn verify_block_commitment(
        el: impl Provider + 'static,
        chain_spec: &ChainSpec<SpecId>,
        n: u64,
    ) {
        // create block commitment to the previous block
        let commit = block_commitment(&el, chain_spec, n).await;

        // preflight the verifier
        let mut env = EthEvmEnv::builder()
            .provider(el)
            .chain_spec(chain_spec)
            .build()
            .await
            .unwrap();
        SteelVerifier::preflight(&mut env)
            .verify(&commit)
            .await
            .unwrap();

        // mock guest execution, by executing the verifier on the GuestEvmEnv
        let env = env.into_input().await.unwrap().into_env(chain_spec);
        SteelVerifier::new(&env).verify(&commit);
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn eip2935_verify_block_commitment() {
        // TODO(https://github.com/foundry-rs/foundry/issues/10357): Use Anvil provider
        let el = ProviderBuilder::new().connect_http(get_el_url());

        verify_block_commitment(el.clone(), &ETH_MAINNET_CHAIN_SPEC, 1).await;
        verify_block_commitment(el.clone(), &ETH_MAINNET_CHAIN_SPEC, 2).await;
        verify_block_commitment(el.clone(), &ETH_MAINNET_CHAIN_SPEC, EIP2935_HISTORY_LIMIT).await;
    }

    #[test(tokio::test)]
    async fn pre_eip2935_verify_block_commitment() {
        let chain_spec = ChainSpec::new_single(31337, SpecId::CANCUN);
        let el = ProviderBuilder::new().connect_anvil_with_config(|conf| conf.cancun());
        el.anvil_mine(Some(HISTORY_LIMIT), None).await.unwrap();

        verify_block_commitment(el.clone(), &chain_spec, 1).await;
        verify_block_commitment(el.clone(), &chain_spec, 2).await;
        verify_block_commitment(el.clone(), &chain_spec, HISTORY_LIMIT).await;
    }

    /// Sends the given transaction and waits for its inclusion.
    async fn send(el: &impl Provider, tx: TransactionRequest) {
        let pending = el.send_transaction(tx).await.unwrap();
        pending.watch().await.unwrap();
    }

    /// Funds the canonical deployer and replays the EIP-2935 deployment transaction.
    ///
    /// Since anvil does not perform the EIP-2935 system call, this mimics a chain right after
    /// fork activation: the contract is deployed, but its ring buffer is completely unset.
    async fn deploy_history_storage(el: &impl Provider) {
        // canonical deployment transaction (https://eips.ethereum.org/EIPS/eip-2935)
        const DEPLOYER: Address = address!("0x3462413Af4609098e1E27A490f554f260213D685");
        static DEPLOY_TX: Bytes = bytes!(
            "f8838085e8d4a510008303d0908080b85c60538060095f395ff33373fffffffffffffffffffffffffffffffffffffffe14604657602036036042575f35600143038111604257611fff81430311604257611fff9006545f5260205ff35b5f5ffd5b5f35611fff600143030655001b820539930aa12693182426612186309f02cfe8a80a0000"
        );

        let funder = el.get_accounts().await.unwrap()[0];
        let tx = TransactionRequest::default()
            .with_from(funder)
            .with_to(DEPLOYER)
            .with_value(U256::from(10u128.pow(18)));
        send(el, tx).await;
        let pending = el.send_raw_transaction(&DEPLOY_TX).await.unwrap();
        pending.watch().await.unwrap();

        let code = el.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
        assert!(!code.is_empty(), "deployment failed");
    }

    /// Records the hash of the latest block in the history storage contract by impersonating
    /// the EIP-2935 system call and returns the corresponding block number.
    async fn record_block_hash(el: &impl Provider) -> BlockNumber {
        const SYSTEM_ADDRESS: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

        let funder = el.get_accounts().await.unwrap()[0];
        let tx = TransactionRequest::default()
            .with_from(funder)
            .with_to(SYSTEM_ADDRESS)
            .with_value(U256::from(10u128.pow(18)));
        send(el, tx).await;
        el.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();

        let number = el.get_block_number().await.unwrap();
        let hash = el
            .get_block_by_number(number.into())
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap()
            .header()
            .hash();
        // the call executes in the next block, storing the hash in its ring buffer slot
        let tx = TransactionRequest::default()
            .with_from(SYSTEM_ADDRESS)
            .with_to(HISTORY_STORAGE_ADDRESS)
            .with_input(Bytes::copy_from_slice(hash.as_slice()));
        send(el, tx).await;

        number
    }

    #[test(tokio::test)]
    async fn eip2935_activation_verify_block_commitment() {
        // the PRAGUE chain spec has `has_eip2935`, but we start Anvil with the CANCUN fork, which
        // does not have the EIP-2935 contract deployed; together with the manual deployment this
        // mimics a chain shortly after fork activation
        let chain_spec = ChainSpec::new_single(31337, SpecId::PRAGUE);
        let el = ProviderBuilder::new().connect_anvil_with_config(|conf| conf.cancun());
        deploy_history_storage(&el).await;
        let recorded = record_block_hash(&el).await;
        el.anvil_mine(Some(HISTORY_LIMIT + 2), None).await.unwrap();

        let latest = el.get_block_number().await.unwrap();
        assert!(latest - recorded > HISTORY_LIMIT);

        // blocks with an unset slot in the BLOCKHASH window are verified via the fallback
        verify_block_commitment(el.clone(), &chain_spec, 2).await;
        verify_block_commitment(el.clone(), &chain_spec, HISTORY_LIMIT).await;
        // the recorded block is verified via the history storage contract
        verify_block_commitment(el.clone(), &chain_spec, latest - recorded).await;

        // blocks beyond the BLOCKHASH window cannot be verified while their slot is unset
        let commit = block_commitment(&el, &chain_spec, latest - recorded + 1).await;
        let mut env = EthEvmEnv::builder()
            .provider(el.clone())
            .chain_spec(&chain_spec)
            .build()
            .await
            .unwrap();
        let err = SteelVerifier::preflight(&mut env)
            .verify(&commit)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid block number"), "{err:#}");
    }

    #[test(tokio::test)]
    #[cfg_attr(
        any(not(feature = "rpc-tests"), no_auth),
        ignore = "RPC tests are disabled"
    )]
    async fn verify_beacon_commitment() {
        let el = ProviderBuilder::new().connect_http(get_el_url());

        // create Beacon commitment from latest block
        let block = el
            .get_block_by_number(AlloyBlockNumberOrTag::Latest)
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header = block.header();
        let commit = Commitment::new(
            CommitmentVersion::Beacon as u16,
            header.timestamp,
            header.parent_beacon_block_root.unwrap(),
            ETH_MAINNET_CHAIN_SPEC.digest(),
        );

        // preflight the verifier
        let mut env = EthEvmEnv::builder()
            .provider(el)
            .chain_spec(&ETH_MAINNET_CHAIN_SPEC)
            .build()
            .await
            .unwrap();
        SteelVerifier::preflight(&mut env)
            .verify(&commit)
            .await
            .unwrap();

        // mock guest execution, by executing the verifier on the GuestEvmEnv
        let env = env
            .into_input()
            .await
            .unwrap()
            .into_env(&ETH_MAINNET_CHAIN_SPEC);
        SteelVerifier::new(&env).verify(&commit);
    }
}
