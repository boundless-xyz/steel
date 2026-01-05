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

use alloy_primitives::{Address, address};
use alloy_sol_types::sol;

sol! {
    interface IERC20 {
        function balanceOf(address account) external view returns (uint);
    }
}

/// Function to call, implements the `SolCall` trait.
pub const CALL: IERC20::balanceOfCall = IERC20::balanceOfCall {
    account: address!("0x8da91A6298eA5d1A8Bc985e99798fd0A0f05701a"),
};

/// Address of the deployed contract to call the function on (USDC contract on Base Mainnet).
pub const CONTRACT: Address = address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

pub const CALLER: Address = Address::ZERO;
