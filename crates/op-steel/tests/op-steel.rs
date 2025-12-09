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

#![cfg(feature = "host")]

use alloy::{
    providers::{Provider, ProviderBuilder},
    uint,
};
use alloy_primitives::{Address, U256, address};
use op_alloy_network::Optimism;
use risc0_op_steel::{
    Contract,
    config::ChainSpec,
    op_revm::OpSpecId,
    optimism::{OpCallError, OpChainSpec, OpEvmEnv},
};
use std::{fmt::Debug, sync::LazyLock};
use test_log::test;

const TEST_CONTRACT: Address = address!("5fbdb2315678afecb367f032d93f642f64180aa3");
alloy::sol!(
    // docker run -i ethereum/solc:0.8.26 - --optimize --bin
    #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506101158061001c5f395ff3fe6080604052348015600e575f80fd5b5060043610603a575f3560e01c80633901104914603e578063b2d14400146051578063dcd9c7fa146062575b5f80fd5b4a60405190815260200160405180910390f35b6060605c36600460c9565b6071565b005b6060606d36600460c9565b6091565b60405163810f002360e01b81526004810182905260240160405180910390fd5b60405181815233907ffceb437c298f40d64702ac26411b2316e79f3c28ffa60edfc891ad4fc8ab82ca9060200160405180910390a250565b5f6020828403121560d8575f80fd5b503591905056fea26469706673582212205791a7ec9e7a4fc4573de6f6d3b007b04764a7d0466e4150702c2ad29184830364736f6c634300081a0033")]
    #[derive(Debug, PartialEq, Eq)]
    contract Test {
        error SomeCustomError(uint256 value);

        event Event(address indexed from, uint256 value);

        /// Emits a test event.
        function testEvent(uint256 value) external {
            emit Event(msg.sender, value);
        }

        /// Emits a test error.
        function testRevertError(uint256 value) external pure {
            revert SomeCustomError(value);
        }

        /// Tests retrieving the [blob base fee](https://eips.ethereum.org/EIPS/eip-4844#gas-accounting).
        function testBlobbasefee() external view returns (uint256) {
            return block.blobbasefee;
        }
    }
);

static OP_ANVIL_CHAIN_SPEC: LazyLock<OpChainSpec> =
    LazyLock::new(|| ChainSpec::new_single(31337, OpSpecId::ISTHMUS.into()));

/// Returns an Anvil provider with the deployed [Test] contract.
async fn test_provider() -> impl Provider<Optimism> + Clone {
    let chain_id = OP_ANVIL_CHAIN_SPEC.chain_id();
    let fork = OP_ANVIL_CHAIN_SPEC.active_fork(0, 0).unwrap();
    let provider = ProviderBuilder::new_with_network()
        .connect_anvil_with_wallet_and_config(|anvil| {
            anvil
                .chain_id(chain_id)
                .args(["--optimism", "--hardfork", &fork.to_string()])
        })
        .unwrap();

    // deploy the test contract
    let instance = Test::deploy(&provider).await.unwrap();
    assert_eq!(*instance.address(), TEST_CONTRACT);

    provider
}

mod event {
    use super::*;
    use risc0_op_steel::{Event, optimism::OpEvmEnv};
    use test_log::test;

    #[test(tokio::test)]
    async fn query_some() {
        let provider = test_provider().await;
        let contract = Test::new(TEST_CONTRACT, &provider);

        const VALUE: U256 = uint!(42_U256);
        // send a transaction to emit an event on chain
        let pending = contract.testEvent(VALUE).send().await.unwrap();
        pending.watch().await.unwrap();

        let mut env = OpEvmEnv::builder()
            .provider(provider)
            .chain_spec(&OP_ANVIL_CHAIN_SPEC)
            .build()
            .await
            .unwrap();

        let preflight_logs = {
            let event =
                risc0_steel::Event::preflight::<Test::Event>(&mut env).address(TEST_CONTRACT);
            event.query().await.unwrap()
        };

        let input = env.into_input().await.unwrap();
        let env = input.into_env(&OP_ANVIL_CHAIN_SPEC);

        let logs = {
            let event = risc0_steel::Event::new::<Test::Event>(&env).address(TEST_CONTRACT);
            event.query()
        };
        assert_eq!(logs, preflight_logs, "mismatch in preflight and execution");
        assert!(
            matches!(
                logs.as_slice(),
                [alloy_primitives::Log {
                    address: TEST_CONTRACT,
                    data: Test::Event { value: VALUE, .. },
                }]
            ),
            "Unexpected event logs: {logs:?}"
        );
    }

    #[test(tokio::test)]
    async fn query_none() {
        let provider = test_provider().await;

        // send a transaction to emit an event on chain
        let contract = Test::deploy(&provider).await.unwrap();
        let pending = contract.testEvent(U256::ZERO).send().await.unwrap();
        pending.watch().await.unwrap();

        let mut env = OpEvmEnv::builder()
            .provider(provider)
            .chain_spec(&OP_ANVIL_CHAIN_SPEC)
            .build()
            .await
            .unwrap();

        let preflight_logs = {
            let event = Event::preflight::<Test::Event>(&mut env).address(TEST_CONTRACT);
            event.query().await.unwrap()
        };

        let input = env.into_input().await.unwrap();
        let env = input.into_env(&OP_ANVIL_CHAIN_SPEC);

        let logs = {
            let event = Event::new::<Test::Event>(&env).address(TEST_CONTRACT);
            event.query()
        };
        assert_eq!(logs, preflight_logs, "mismatch in preflight and execution");
        assert!(logs.is_empty());
    }
}

#[test(tokio::test)]
async fn revert_error() {
    const VALUE: U256 = uint!(42_U256);
    const CALL: Test::testRevertErrorCall = Test::testRevertErrorCall { value: VALUE };

    let provider = test_provider().await;
    let mut env = OpEvmEnv::builder()
        .provider(provider)
        .chain_spec(&OP_ANVIL_CHAIN_SPEC)
        .build()
        .await
        .unwrap();

    // should return an OpCallError inside an `anyhow::Error`
    Contract::preflight(TEST_CONTRACT, &mut env)
        .call_builder(&CALL)
        .call()
        .await
        .unwrap_err()
        .downcast_ref::<OpCallError>()
        .unwrap();

    let input = env.into_input().await.unwrap();
    let env = input.into_env(&OP_ANVIL_CHAIN_SPEC);

    let err = Contract::new(TEST_CONTRACT, &env)
        .call_builder(&CALL)
        .try_call()
        .unwrap_err();
    assert_eq!(
        err.as_decoded_error::<Test::SomeCustomError>(),
        Some(Test::SomeCustomError { value: VALUE }),
        "Unexpected error: {err}"
    );
}

#[test(tokio::test)]
async fn blobbasefee() -> anyhow::Result<()> {
    const CALL: Test::testBlobbasefeeCall = Test::testBlobbasefeeCall {};

    let mut env = OpEvmEnv::builder()
        .provider(test_provider().await)
        .chain_spec(&OP_ANVIL_CHAIN_SPEC)
        .build()
        .await?;

    Contract::preflight(TEST_CONTRACT, &mut env)
        .call_builder(&CALL)
        .call()
        .await?;

    let input = env.into_input().await?;
    let env = input.into_env(&OP_ANVIL_CHAIN_SPEC);

    let result = Contract::new(TEST_CONTRACT, &env)
        .call_builder(&CALL)
        .call();
    // Returns a fixed value of 1 wei. (L2 blocks do not have their own blob markets).
    assert_eq!(result, uint!(1_U256));

    Ok(())
}
