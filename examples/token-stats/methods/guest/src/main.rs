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

use alloy_sol_types::SolValue;
use risc0_steel::{
    Contract,
    ethereum::{ETH_MAINNET_CHAIN_SPEC, EthMultiblockEvmInput},
};
use risc0_zkvm::guest::env;
use token_stats_core::{APRCommitment, BLOCK_INTERVAL, CONTRACT, CometMainInterface};

const SECONDS_PER_YEAR: u128 = 60 * 60 * 24 * 365;

fn main() {
    // Read the multiblock input from the guest environment.
    let input: EthMultiblockEvmInput = env::read();

    // Converts the input into a `EvmEnv` for execution.
    let envs = input.into_env(&ETH_MAINNET_CHAIN_SPEC);

    // Require at least two samples so the committed APR is a genuine average over a positive
    // number of days, rather than a single instantaneous rate.
    let numbers: Vec<_> = envs.block_numbers().collect();
    assert!(numbers.len() >= 2, "at least two sampled blocks are required");
    // Check that the EVM states are exactly BLOCK_INTERVAL blocks apart.
    for window in numbers.windows(2) {
        assert_eq!(window[1] - window[0], BLOCK_INTERVAL);
    }

    // Execute the view calls on each EVM state.
    let rates: Vec<u64> = envs
        .iter()
        .map(|env| {
            let contract = Contract::new(CONTRACT, env);
            let utilization = contract
                .call_builder(&CometMainInterface::getUtilizationCall {})
                .call();
            contract
                .call_builder(&CometMainInterface::getSupplyRateCall { utilization })
                .call()
        })
        .collect();

    // The formula for APR in percentage is the following:
    // Seconds Per Year = 60 * 60 * 24 * 365
    // Utilization = getUtilization()
    // Supply Rate = getSupplyRate(Utilization)
    // Supply APR = Supply Rate / (10 ^ 18) * Seconds Per Year * 100
    let sum: u128 = rates.iter().map(|&r| r as u128).sum();
    let annual_supply_rate = u64::try_from(sum * SECONDS_PER_YEAR / rates.len() as u128)
        .expect("annual supply rate fits in u64");

    // This commits the APR at current utilization rate for this given block.
    let journal = APRCommitment {
        numDays: (rates.len() - 1) as u64,
        finalBlockNumber: envs.last().header().number,
        annualSupplyRate: annual_supply_rate,
        commitment: envs.into_commitment(),
    };
    env::commit_slice(&journal.abi_encode());
}
