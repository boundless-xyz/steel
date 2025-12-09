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

use alloy_primitives::{Address, ChainId};
use alloy_sol_types::{SolValue, sol};
use risc0_steel::{Commitment, ethereum::EthEvmInput};
use serde::{Deserialize, Serialize};

sol! {
    /// Interface to be called by the guest.
    interface IERC20 {
        function balanceOf(address account) external view returns (uint);
    }
}

sol! {
    /// Data committed to by the guest.
    #[sol(all_derives)]
    struct Journal {
        Commitment commitment;
        address contract;
    }
}

impl Journal {
    /// ABI-encodes the journal into a byte vector according to Solidity ABI specifications.
    #[inline]
    pub fn abi_encode(&self) -> Vec<u8> {
        SolValue::abi_encode(self)
    }
}

/// Input for the guest
#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    pub chain_id: ChainId,
    pub evm_input: EthEvmInput,
    pub erc20_contract: Address,
    pub account: Address,
}
