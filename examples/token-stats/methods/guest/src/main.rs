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
    Contract, SteelVerifier,
    ethereum::{ETH_MAINNET_CHAIN_SPEC, EthMultiblockEvmInput},
};
use risc0_zkvm::guest::env;
use token_stats_core::{APRCommitment, CONTRACT, CometMainInterface};

const SECONDS_PER_YEAR: u128 = 60 * 60 * 24 * 365;

fn main() {
    // Read the first input from the guest environment. It corresponds to the older EVM state.
    let input: EthMultiblockEvmInput = env::read();

    // Converts the input into a `EvmEnv` for execution.
    let envs = input.into_env(&ETH_MAINNET_CHAIN_SPEC);

    // Check that the EVM states are exactly 7200 blocks apart.
    for (env_prev, env) in envs.iter().zip(envs.iter().skip(1)) {
        assert_eq!(env.header().number - env_prev.header().number, 7200);
    }

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
    let annual_supply_rate_u128 =
        rates.iter().map(|&r| r as u128).sum::<u128>() * SECONDS_PER_YEAR / rates.len() as u128;
    let annual_supply_rate = u64::try_from(annual_supply_rate_u128).unwrap();

    // This commits the APR at current utilization rate for this given block.
    let journal = APRCommitment {
        days: rates.len() as u64,
        finalBlockNumber: envs.last().header().number,
        annualSupplyRate: annual_supply_rate,
        commitment: envs.into_commitment(),
    };
    env::commit_slice(&journal.abi_encode());
}
