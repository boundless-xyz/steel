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
use anyhow::ensure;

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
                // use history storage contract when EIP-2935 was activated
                let block_hash = if self.env.spec_id.has_eip2935() {
                    // history storage contract reverts when `id` id not in allowed history window
                    HistoryStorageContract::new(self.env).call(id)
                } else {
                    let block_number = validate_history_window(self.env.header().inner(), id, 256)
                        .expect("Invalid block number");
                    self.env.db().block_hash(block_number)
                };
                assert_eq!(block_hash, commitment.digest, "Invalid block hash");
            }
            Some(CommitmentVersion::Beacon) => {
                assert!(self.env.spec_id.has_eip4788(), "EIP-4788 required");
                // beacon roots contract reverts when `id` id not in allowed history window
                let beacon_root = BeaconRootsContract::new(self.env).call(id);
                assert_eq!(beacon_root, commitment.digest, "Invalid beacon root");
            }
            _ => {
                unimplemented!("Unsupported version: {:x}", version_code)
            }
        }
    }
}

#[cfg(feature = "host")]
mod host {
    use super::*;
    use crate::host::{HostEvmEnv, db::ProviderDb};
    use alloy::providers::{Network, Provider};
    use alloy_eips::eip2935;
    use anyhow::Context;
    use revm::Database;

    impl<'a, F, N, P, C> SteelVerifier<&'a mut HostEvmEnv<ProviderDb<N, P>, F, C>>
    where
        F: EvmFactory,
        N: Network,
        P: Provider<N> + Send + Sync + 'static,
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
            self,
            commitment: &Commitment,
            config_id: B256,
        ) -> anyhow::Result<()> {
            log::debug!("Executing preflight verifying {commitment:?}");

            ensure!(commitment.configID == config_id, "invalid config ID");
            let (id, version_code) = commitment.decode_id();
            match CommitmentVersion::n(version_code) {
                Some(CommitmentVersion::Block) => {
                    let header = self.env.header().inner();
                    let block_hash = if self.env.spec_id.has_eip2935() {
                        validate_history_window(header, id, eip2935::HISTORY_SERVE_WINDOW as u64)
                            .context("invalid block number")?;
                        HistoryStorageContract::preflight(self.env).call(id).await?
                    } else {
                        let block_number = validate_history_window(header, id, 256)
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
                    ensure!(beacon_root == commitment.digest, "invalid beacon root");

                    Ok(())
                }
                version => unimplemented!(
                    "Unsupported commitment version: {}",
                    version.map_or(format!("Unknown({version_code:x})"), |v| format!("{v:?}"))
                ),
            }
        }
    }
}

fn validate_history_window(
    header: &impl EvmBlockHeader,
    block_number: U256,
    windows: u64,
) -> anyhow::Result<u64> {
    let block_number: BlockNumber = block_number.saturating_to();
    ensure!(
        block_number < header.number() && header.number() - block_number <= windows,
        "only valid for the {windows} most recent blocks, excluding the current one"
    );

    Ok(block_number)
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
        network::{BlockResponse, primitives::HeaderResponse},
        providers::{Provider, ProviderBuilder, ext::AnvilApi},
        rpc::types::BlockNumberOrTag as AlloyBlockNumberOrTag,
    };
    use revm::primitives::hardfork::SpecId;
    use test_log::test;

    async fn verify_block_commitment(
        el: impl Provider + 'static,
        chain_spec: &ChainSpec<SpecId>,
        n: u64,
    ) {
        // create block commitment to the previous block
        let latest = el.get_block_number().await.unwrap();
        let block = el
            .get_block_by_number((latest - n).into())
            .await
            .expect("eth_getBlockByNumber failed")
            .unwrap();
        let header = block.header();
        let commit = Commitment::new(
            CommitmentVersion::Block as u16,
            header.number(),
            header.hash(),
            chain_spec.digest(),
        );

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
        // TODO: Make this an Anvil provider, once Anvil has EIP-2935 support
        let el = ProviderBuilder::new().connect_http(get_el_url());

        verify_block_commitment(el.clone(), &ETH_MAINNET_CHAIN_SPEC, 1).await;
        verify_block_commitment(el.clone(), &ETH_MAINNET_CHAIN_SPEC, 8191).await;
    }

    #[test(tokio::test)]
    async fn pre_eip2935_verify_block_commitment() {
        let chain_spec = ChainSpec::new_single(31337, SpecId::CANCUN);
        let el = ProviderBuilder::new().connect_anvil_with_config(|conf| conf.cancun());
        el.anvil_mine(Some(256), None).await.unwrap();

        verify_block_commitment(el.clone(), &chain_spec, 1).await;
        verify_block_commitment(el.clone(), &chain_spec, 256).await;
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
