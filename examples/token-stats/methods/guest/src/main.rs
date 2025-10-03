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

use alloy_sol_types::SolValue;
use risc0_steel::{
    ethereum::{EthMultiblockEvmInput, ETH_MAINNET_CHAIN_SPEC},
    Contract,
};
use risc0_zkvm::guest::env;
use token_stats_core::{APRCommitment, CometMainInterface, CONTRACT};

const SECONDS_PER_YEAR: u64 = 60 * 60 * 24 * 365;

fn main() {
    // Read the first input from the guest environment. It corresponds to the older EVM state.
    let input: EthMultiblockEvmInput = env::read();

    // Converts the input into a `EvmEnv` for execution.
    let envs = input.into_env(&ETH_MAINNET_CHAIN_SPEC);
    // Check that there are exactly two EVM states.
    assert_eq!(envs.len(), 2);

    // Execute the view calls on each EVM state.
    let rates = envs
        .iter()
        .map(|env| {
            // Execute the view calls on the older EVM state.
            let contract = Contract::new(CONTRACT, env);
            let utilization = contract
                .call_builder(&CometMainInterface::getUtilizationCall {})
                .call();
            contract
                .call_builder(&CometMainInterface::getSupplyRateCall { utilization })
                .call()
        })
        .collect::<Vec<_>>();

    // The formula for APR in percentage is the following:
    // Seconds Per Year = 60 * 60 * 24 * 365
    // Utilization = getUtilization()
    // Supply Rate = getSupplyRate(Utilization)
    // Supply APR = Supply Rate / (10 ^ 18) * Seconds Per Year * 100
    //
    // Compute the average APR, by computing the average over both states.
    let annual_supply_rate = rates.iter().sum::<u64>() * SECONDS_PER_YEAR / rates.len() as u64;

    // This commits the APR at current utilization rate for this given block.
    let journal = APRCommitment {
        commitment: envs.into_commitment(),
        annualSupplyRate: annual_supply_rate,
    };
    env::commit_slice(&journal.abi_encode());
}
