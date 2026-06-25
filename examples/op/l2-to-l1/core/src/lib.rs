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
    account: address!("0x38d693cE1dF5AaDF7bC62595A37D667aD57922e5"),
};

/// Address of the deployed contract to call the function on (USDC contract on OP Mainnet).
pub const CONTRACT: Address = address!("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85");

pub const CALLER: Address = Address::ZERO;
