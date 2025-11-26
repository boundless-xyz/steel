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
//
// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.26;

import {Script} from "forge-std/Script.sol";
import {console2} from "forge-std/console2.sol";
import {Counter} from "../src/Counter.sol";
import {ImageID} from "../src/ImageID.sol";
import {IRiscZeroVerifier} from "risc0-ethereum/IRiscZeroVerifier.sol";

// Mocks
import {FixedSupplyToken} from "../test/utils/FixedSupplyToken.sol";
import {MockVerifier} from "../test/utils/MockVerifier.sol";

contract DeployCounter is Script {
    function run() external {
        vm.startBroadcast();

        console2.log(unicode"⚠️  Deploying Local Development Environment");

        // Deploy Mock Token that mints 1000 tokens to the deployer (msg.sender)
        FixedSupplyToken token = new FixedSupplyToken("Toyken", "TOY", msg.sender);
        console2.log("Deployed ERC20 TOKEN to", address(token));
        console2.log("Account", msg.sender, "has balance:", token.balanceOf(msg.sender));

        // Deploy Mock Verifier, that accepts dev mode proofs with the selector 0xFFFFFFFF
        IRiscZeroVerifier verifier = new MockVerifier();
        console2.log("Deployed RiscZeroMockVerifier to", address(verifier));

        // Deploy application contract.
        Counter counter = new Counter(verifier, address(token), ImageID.ERC20_COUNTER_GUEST_ID);
        console2.log("Deployed Counter to", address(counter));

        vm.stopBroadcast();
    }
}
