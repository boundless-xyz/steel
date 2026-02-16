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

#![cfg(feature = "host")]

use std::fmt::Debug;

use alloy::{
    providers::{Provider, ProviderBuilder, ext::AnvilApi},
    rpc::types::TransactionRequest,
    uint,
};
use alloy_primitives::{Address, U256, address, b256, keccak256};
use alloy_sol_types::SolCall;
use alloy_trie::EMPTY_ROOT_HASH;
use common::CallOptions;
use revm::context::result::HaltReason;
use risc0_steel::{
    Account, CallError, EvmBlockHeader, SyncEnv,
    account::AccountInfo,
    ethereum::{EthCallError, EthEvmEnv, STEEL_TEST_PRAGUE_CHAIN_SPEC},
};
use test_log::test;

mod common;

const STEEL_TEST_CONTRACT: Address = address!("5fbdb2315678afecb367f032d93f642f64180aa3");
alloy::sol!(
    // docker run -i ethereum/solc:0.8.26 - --optimize --bin
    #[sol(rpc, bytecode="60e060405234801561000f575f80fd5b505f60405161001d906100c4565b908152602001604051809103905ff08015801561003c573d5f803e3d5ffd5b506001600160a01b0316608052604051602a90610058906100c4565b908152602001604051809103905ff080158015610077573d5f803e3d5ffd5b506001600160a01b031660a052604051602a90610093906100c4565b908152602001604051809103905ff0801580156100b2573d5f803e3d5ffd5b506001600160a01b031660c0526100d0565b60e0806106c883390190565b60805160a05160c0516105c76101015f395f6101a701525f61022701525f81816102a7015261032701526105c75ff3fe608060405234801561000f575f80fd5b50600436106100e5575f3560e01c80637d732b5f11610088578063b2d1440011610063578063b2d1440014610158578063d62f7a421461016b578063dcd9c7fa1461017e578063fb66703614610191575f80fd5b80637d732b5f1461013d5780639094020d14610143578063ab8fd80c1461014d575f80fd5b806330e49663116100c357806330e49663146101205780634131718514610126578063583155d81461012d5780637023922214610135575f80fd5b80630692d13c146100e9578063163e004a146100ff5780632e8bde3914610106575b5f80fd5b5f3b5b6040519081526020015b60405180910390f35b5f546100ec565b325b6040516001600160a01b0390911681526020016100f6565b3a6100ec565b443b6100ec565b6100ec6101a4565b6100ec6103c8565b466100ec565b61014b61040a565b005b4360ff1901406100ec565b61014b6101663660046104d9565b610410565b6101086101793660046104f0565b610430565b61014b61018c3660046104d9565b610495565b61014b61019f36600461052f565b6104cd565b5f7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b03166395cacbe06040518163ffffffff1660e01b8152600401602060405180830381865afa158015610201573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906102259190610555565b7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b031663c82fdf366040518163ffffffff1660e01b8152600401602060405180830381865afa158015610281573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906102a59190610555565b7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b03166395cacbe06040518163ffffffff1660e01b8152600401602060405180830381865afa158015610301573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906103259190610555565b7f00000000000000000000000000000000000000000000000000000000000000006001600160a01b031663c82fdf366040518163ffffffff1660e01b8152600401602060405180830381865afa158015610381573d5f803e3d5ffd5b505050506040513d601f19601f820116820180604052508101906103a59190610555565b6103af919061056c565b6103b9919061056c565b6103c3919061056c565b905090565b6040515f906002906020818481855afa1580156103e7573d5f803e3d5ffd5b5050506040513d601f19601f820116820180604052508101906103c39190610555565b5b61040b565b60405163810f002360e01b81526004810182905260240160405180910390fd5b604080515f8082526020820180845287905260ff861692820192909252606081018490526080810183905260019060a0016020604051602081039080840390855afa158015610481573d5f803e3d5ffd5b5050604051601f1901519695505050505050565b60405181815233907ffceb437c298f40d64702ac26411b2316e79f3c28ffa60edfc891ad4fc8ab82ca9060200160405180910390a250565b806104d6575f80fd5b50565b5f602082840312156104e9575f80fd5b5035919050565b5f805f8060808587031215610503575f80fd5b84359350602085013560ff8116811461051a575f80fd5b93969395505050506040820135916060013590565b5f6020828403121561053f575f80fd5b8135801515811461054e575f80fd5b9392505050565b5f60208284031215610565575f80fd5b5051919050565b8082018082111561058b57634e487b7160e01b5f52601160045260245ffd5b9291505056fea2646970667358221220fc4c1ad2a5666b862737630eec3bfc359480b1cff69465ef3bfb2d120e41042864736f6c634300081a00336080604052348015600e575f80fd5b5060405160e038038060e08339810160408190526029916034565b5f819055600155604a565b5f602082840312156043575f80fd5b5051919050565b608b8060555f395ff3fe6080604052348015600e575f80fd5b50600436106030575f3560e01c806395cacbe0146034578063c82fdf3614604e575b5f80fd5b603c60015481565b60405190815260200160405180910390f35b603c5f548156fea2646970667358221220c315a9368a8e8b62edb25cd12c69bd86898dac6e7dc1e7d3fbc6931b6708a66c64736f6c634300081a0033")]
    #[derive(Debug, PartialEq, Eq)]
    contract SteelTest {
        Value internal immutable VALUE0;
        Value internal immutable VALUE42A;
        Value internal immutable VALUE42B;

        error SomeCustomError(uint256 value);

        event Event(address indexed from, uint256 value);

        constructor() {
            VALUE0 = new Value(0);
            VALUE42A = new Value(42);
            VALUE42B = new Value(42);
        }

        /// Emits a test event.
        function testEvent(uint256 value) external {
            emit Event(msg.sender, value);
        }

        /// Emits a test error.
        function testRevertError(uint256 value) external pure {
            revert SomeCustomError(value);
        }

        /// Tests a function that does not call return.
        function testRequire(bool value) external pure {
            require(value);
        }

        /// Tests the ecRecover precompile.
        function testECRecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
            return ecrecover(hash, v, r, s);
        }

        /// Tests the SHA256 precompile.
        function testSHA256() external pure returns (bytes32) {
            return sha256("");
        }

        /// Tests accessing the code of a nonexistent account.
        function testNonexistentAccount() external view returns (uint256 size) {
            address a = address(uint160(block.prevrandao));
            assembly { size := extcodesize(a) }
        }

        /// Tests accessing the code of the EOA account 0x0000000000000000000000000000000000000000.
        function testEoaAccount() external view returns (uint256 size) {
            assembly { size := extcodesize(0) }
        }

        /// Tests the blockhash opcode.
        function testBlockhash() external view returns (bytes32 h) {
            assembly { h := blockhash(sub(number(), 256)) }
        }

        /// Tests retrieving the chain ID.
        function testChainid() external view returns (uint256) {
            return block.chainid;
        }

        /// Tests retrieving the address of the sender of the transaction.
        function testOrigin() external view returns (address) {
            return tx.origin;
        }

        /// Tests retrieving the gas price.
        function testGasprice() external view returns (uint256) {
            return tx.gasprice;
        }

        /// Tests loading a word from storage of an account with empty storage.
        function testLoadEmptyStorage() external view returns (uint256 val) {
            assembly { val := sload(0) }
        }

        /// Tests calling multiple contracts with the same and different storage.
        function testMultiContractCalls() external view returns (uint256) {
            return VALUE0.val1() + VALUE0.val2() + VALUE42A.val1() + VALUE42B.val2();
        }

        /// Infinite loop to burn gas.
        function testBurnGas() external pure {
            while (true) {}
        }
    }

    contract Value {
        uint256 public val1;
        uint256 public val2;

        constructor(uint256 _value) {
            val1 = _value;
            val2 = _value;
        }
    }
);

/// Returns an Anvil provider with the deployed [SteelTest] contract.
async fn test_provider() -> impl Provider + Clone {
    let chain_id = STEEL_TEST_PRAGUE_CHAIN_SPEC.chain_id();
    let fork = STEEL_TEST_PRAGUE_CHAIN_SPEC.active_fork(0, 0).unwrap();
    let provider = ProviderBuilder::new()
        .connect_anvil_with_wallet_and_config(|anvil| {
            anvil
                .chain_id(chain_id)
                .args(["--hardfork", &fork.to_string()])
        })
        .unwrap();
    let node_info = provider.anvil_node_info().await.unwrap();
    log::info!("Anvil started: {node_info:?}");
    let instance = SteelTest::deploy(&provider).await.unwrap();
    assert_eq!(*instance.address(), STEEL_TEST_CONTRACT);

    provider
}

async fn account_query<P>(provider: P, address: Address, bytecode: bool) -> AccountInfo
where
    P: Provider + 'static,
{
    let mut env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
        .build()
        .await
        .unwrap();
    let block_hash = env.header().seal();
    let block_number = env.header().number;

    let preflight_info = {
        let account = Account::preflight(address, &mut env).bytecode(bytecode);
        account.info().await.unwrap()
    };

    let input = env.into_input().await.unwrap();
    let env = input.into_env(&STEEL_TEST_PRAGUE_CHAIN_SPEC);
    let commitment = env.commitment();
    assert_eq!(commitment.digest, block_hash, "invalid commitment");
    assert_eq!(
        commitment.id,
        U256::from(block_number),
        "invalid commitment"
    );

    let info = {
        let account = Account::new(address, &env).bytecode(bytecode);
        account.info()
    };
    assert_eq!(info, preflight_info, "mismatch in preflight and execution");

    info
}

#[test(tokio::test)]
async fn account_info() {
    let p = test_provider().await;
    let address = STEEL_TEST_CONTRACT;
    let info = account_query(p.clone(), address, false).await;

    assert_eq!(info.nonce, p.get_transaction_count(address).await.unwrap());
    assert_eq!(info.balance, p.get_balance(address).await.unwrap());
    assert_eq!(info.storage_root, EMPTY_ROOT_HASH);
    let code = p.get_code_at(address).await.unwrap();
    assert_eq!(info.code_hash, keccak256(&code));
    assert_eq!(info.code, None);
}

#[test(tokio::test)]
async fn account_info_with_bytecode() {
    let p = test_provider().await;
    let address = STEEL_TEST_CONTRACT;
    let info = account_query(p.clone(), address, true).await;

    assert_eq!(info.nonce, p.get_transaction_count(address).await.unwrap());
    assert_eq!(info.balance, p.get_balance(address).await.unwrap());
    assert_eq!(info.storage_root, EMPTY_ROOT_HASH);
    let code = p.get_code_at(address).await.unwrap();
    assert_eq!(info.code_hash, keccak256(&code));
    assert_eq!(info.code, Some(code));
}

mod event {
    use super::*;
    use risc0_steel::Event;
    use test_log::test;

    #[test(tokio::test)]
    async fn event_query_some() {
        let provider = test_provider().await;
        let contract = SteelTest::new(STEEL_TEST_CONTRACT, &provider);

        const VALUE: U256 = uint!(42_U256);
        // send a transaction to emit an event on chain
        let pending = contract.testEvent(VALUE).send().await.unwrap();
        pending.watch().await.unwrap();

        let mut env = EthEvmEnv::builder()
            .provider(provider)
            .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
            .build()
            .await
            .unwrap();

        let preflight_logs = {
            let event = risc0_steel::Event::preflight::<SteelTest::Event>(&mut env)
                .address(STEEL_TEST_CONTRACT);
            event.query().await.unwrap()
        };

        let input = env.into_input().await.unwrap();
        let env = input.into_env(&STEEL_TEST_PRAGUE_CHAIN_SPEC);

        let logs = {
            let event =
                risc0_steel::Event::new::<SteelTest::Event>(&env).address(STEEL_TEST_CONTRACT);
            event.query()
        };
        assert_eq!(logs, preflight_logs, "mismatch in preflight and execution");
        assert!(
            matches!(
                logs.as_slice(),
                [alloy_primitives::Log {
                    address: STEEL_TEST_CONTRACT,
                    data: SteelTest::Event { value: VALUE, .. },
                }]
            ),
            "Unexpected event logs: {logs:?}"
        );
    }

    #[test(tokio::test)]
    async fn event_query_none() {
        let provider = test_provider().await;

        // send a transaction to emit an event on chain
        let contract = SteelTest::deploy(&provider).await.unwrap();
        let pending = contract.testEvent(U256::ZERO).send().await.unwrap();
        pending.watch().await.unwrap();

        let mut env = EthEvmEnv::builder()
            .provider(provider)
            .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
            .build()
            .await
            .unwrap();

        let preflight_logs = {
            let event = Event::preflight::<SteelTest::Event>(&mut env).address(STEEL_TEST_CONTRACT);
            event.query().await.unwrap()
        };

        let input = env.into_input().await.unwrap();
        let env = input.into_env(&STEEL_TEST_PRAGUE_CHAIN_SPEC);

        let logs = {
            let event = Event::new::<SteelTest::Event>(&env).address(STEEL_TEST_CONTRACT);
            event.query()
        };
        assert_eq!(logs, preflight_logs, "mismatch in preflight and execution");
        assert!(logs.is_empty());
    }
}

#[test(tokio::test)]
async fn revert_error() {
    const VALUE: U256 = uint!(42_U256);

    let err = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testRevertErrorCall { value: VALUE },
        CallOptions::new(),
    )
    .await
    .expect_err("should revert");
    assert!(
        matches!(
            err.downcast_ref::<EthCallError>()
                .and_then(CallError::as_decoded_error::<SteelTest::SomeCustomError>),
            Some(SteelTest::SomeCustomError { value: VALUE })
        ),
        "Unexpected error: {err:#}"
    );
}

#[test(tokio::test)]
async fn require() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testRequireCall { value: true },
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, ().into());

    let err = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testRequireCall { value: false },
        CallOptions::new(),
    )
    .await
    .expect_err("should revert");
    assert!(
        matches!(
            err.downcast_ref::<EthCallError>(),
            Some(CallError::Reverted(data)) if data.is_empty()
        ),
        "Unexpected error: {err:#}"
    );
}

#[test(tokio::test)]
async fn ec_recover() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testECRecoverCall {
            hash: b256!("385967023fb9520b497ee37da9c1e3d5faac1385800ce4ed07ca32d7893c7bb5"),
            v: 27,
            r: b256!("905eadefa07b89ede807aee158ad7ef0414838a9c084e4192029e0383d000b84"),
            s: b256!("250f8aab57d60992fd1fa4fd681491575e74b1c5691ebc631ac2326beb23c5c7"),
        },
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, address!("328809Bc894f92807417D2dAD6b7C998c1aFdac6"));
}

#[test(tokio::test)]
async fn sha256() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testSHA256Call {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        result,
        b256!("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}

#[test(tokio::test)]
async fn nonexistent_account() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testNonexistentAccountCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, uint!(0_U256));
}

#[test(tokio::test)]
async fn eoa_account() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testEoaAccountCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, uint!(0_U256));
}

#[test(tokio::test)]
async fn blockhash() {
    let provider = test_provider().await;
    let block_hash = provider.anvil_node_info().await.unwrap().current_block_hash;
    // mine more blocks to assure that the chain is long enough
    provider.anvil_mine(Some(256), None).await.unwrap();

    let result = common::eth_call(
        provider,
        STEEL_TEST_CONTRACT,
        SteelTest::testBlockhashCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, block_hash);
}

#[test(tokio::test)]
async fn chainid() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testChainidCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, U256::from(STEEL_TEST_PRAGUE_CHAIN_SPEC.chain_id()));
}

#[test(tokio::test)]
async fn origin() {
    let from = address!("0000000000000000000000000000000000000042");
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testOriginCall {},
        CallOptions::with_from(from),
    )
    .await
    .unwrap();
    assert_eq!(result, from);
}

#[test(tokio::test)]
async fn gasprice() {
    let gas_price = 42;
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testGaspriceCall {},
        CallOptions::with_gas_price(gas_price),
    )
    .await
    .unwrap();
    assert_eq!(result, U256::from(gas_price));
}

#[test(tokio::test)]
async fn load_empty_storage() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testLoadEmptyStorageCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, uint!(0_U256));
}

#[test(tokio::test)]
async fn multi_contract_calls() {
    let result = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testMultiContractCallsCall {},
        CallOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, uint!(84_U256));
}

#[test(tokio::test)]
async fn call_eoa() {
    let err = common::eth_call(
        test_provider().await,
        Address::ZERO,
        SteelTest::testBlockhashCall {},
        CallOptions::new(),
    )
    .await
    .expect_err("calling an EOA should fail");
    assert!(
        matches!(
            err.downcast_ref::<EthCallError>(),
            Some(CallError::NoReturn)
        ),
        "Unexpected error: {err:#}"
    );
}

#[test(tokio::test)]
async fn out_of_gas() {
    let err = common::eth_call(
        test_provider().await,
        STEEL_TEST_CONTRACT,
        SteelTest::testBurnGasCall {},
        CallOptions::with_gas(30_000_000),
    )
    .await
    .expect_err("calling an EOA should fail");
    assert!(
        matches!(
            err.downcast_ref::<EthCallError>(),
            Some(CallError::Halted(HaltReason::OutOfGas(_)))
        ),
        "Unexpected error: {err:#}"
    );
}

#[test(tokio::test)]
async fn no_preflight() {
    let env = EthEvmEnv::builder()
        .provider(test_provider().await)
        .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
        .build()
        .await
        .unwrap();
    match env.into_input().await {
        Ok(_) => panic!("calling into_input without a preflight should fail"),
        Err(err) => assert_eq!(
            err.to_string(),
            "no accounts accessed: use Contract::preflight"
        ),
    }
}

alloy::sol!(
    // docker run -i ethereum/solc:0.8.26 - --optimize --bin
    #[sol(rpc, bytecode="60a0604052348015600e575f80fd5b5060405161012a38038061012a833981016040819052602b91604b565b60808190525f5b6080518110156045576001808255016032565b50506061565b5f60208284031215605a575f80fd5b5051919050565b60805160b46100765f395f6047015260b45ff3fe6080604052348015600e575f80fd5b50600436106026575f3560e01c8063380eb4e014602a575b5f80fd5b60306042565b60405190815260200160405180910390f35b5f805b7f0000000000000000000000000000000000000000000000000000000000000000811015607a57805491909101906001016045565b509056fea26469706673582212203687b75eefdd9cc7ceedb243aa360bd9e1b4cab1930149a371efef74ce18bdf164736f6c634300081a0033")]
    #[derive(Debug, PartialEq, Eq)]
    contract SlotsTest {
        uint256 internal immutable N;

        constructor(uint256 n) {
            N = n;
            for (uint256 i = 0; i < N; i++) {
                assembly { sstore(i, 1) }
            }
        }

        function sload() external view returns (uint256 sum) {
            for (uint256 i = 0; i < N; i++) {
                assembly { sum := add(sum, sload(i)) }
            }
        }
    }
);

#[test(tokio::test)]
async fn prefetch_access_list() {
    const NUM_SLOTS: U256 = uint!(1_250_U256);

    let provider = test_provider().await;
    let instance = SlotsTest::deploy(&provider, NUM_SLOTS).await.unwrap();
    let address = *instance.address();
    let call = SlotsTest::sloadCall {};

    let mut access_list = {
        let tx = TransactionRequest::default()
            .from(address)
            .to(address)
            .input(call.abi_encode().into());
        let access_list_with_gas_used = provider.create_access_list(&tx).await.unwrap();
        access_list_with_gas_used.access_list
    };
    // remove one storage proof from the access list
    access_list.0.first_mut().unwrap().storage_keys.pop();
    let options = CallOptions::with_access_list(access_list);

    common::eth_call(provider, address, call, options)
        .await
        .unwrap();
}

/// Shared validation logic that runs against any SyncEnv.
fn sync_validation(env: &mut impl SyncEnv, contract: Address) -> (U256, U256, U256, u64, u64) {
    let chainid: U256 = env.call(contract, &SteelTest::testChainidCall {});
    let multi: U256 = env.call(contract, &SteelTest::testMultiContractCallsCall {});
    let storage: U256 = env.call(contract, &SteelTest::testLoadEmptyStorageCall {});
    let timestamp = env.header().timestamp();
    let number = env.header().number();

    (chainid, multi, storage, timestamp, number)
}

/// Tests that SyncEnv produces identical results on host and guest environments.
#[test(tokio::test(flavor = "multi_thread"))]
async fn sync_env_host_guest_match() {
    let provider = test_provider().await;

    let mut host_env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
        .build()
        .await
        .unwrap();

    let host_result = sync_validation(&mut host_env, STEEL_TEST_CONTRACT);

    let input = host_env.into_input().await.unwrap();
    let mut guest_env = input.into_env(&STEEL_TEST_PRAGUE_CHAIN_SPEC);

    let guest_result = sync_validation(&mut guest_env, STEEL_TEST_CONTRACT);

    assert_eq!(host_result, guest_result, "host and guest results differ");

    let (chainid, multi, storage, timestamp, number) = guest_result;
    assert_eq!(
        chainid,
        U256::from(STEEL_TEST_PRAGUE_CHAIN_SPEC.chain_id()),
        "chainid mismatch"
    );
    assert_eq!(multi, uint!(84_U256), "multi-contract call mismatch");
    assert_eq!(storage, uint!(0_U256), "empty storage mismatch");
    assert!(timestamp > 0, "timestamp should be non-zero");
    assert!(number > 0, "block number should be non-zero");
}

/// Tests that SyncEnv::try_call returns an error (not a panic) for a call to an EOA.
#[test(tokio::test(flavor = "multi_thread"))]
async fn sync_env_try_call_error() {
    let provider = test_provider().await;

    let mut host_env = EthEvmEnv::builder()
        .provider(provider)
        .chain_spec(&STEEL_TEST_PRAGUE_CHAIN_SPEC)
        .build()
        .await
        .unwrap();

    let result = host_env.try_call(Address::ZERO, &SteelTest::testChainidCall {});
    assert!(result.is_err(), "calling an EOA should fail");
}
