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

import {Test} from "forge-std/Test.sol";
import {Receipt as RiscZeroReceipt} from "risc0-ethereum/IRiscZeroVerifier.sol";
import {RiscZeroMockVerifier} from "risc0-ethereum/test/RiscZeroMockVerifier.sol";
import {Counter} from "../src/Counter.sol";
import {Steel, Encoding, ChainSpec} from "risc0-steel/Steel.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract ERC20FixedSupply is ERC20 {
    constructor(string memory name, string memory symbol, address owner) ERC20(name, symbol) {
        _mint(owner, 1000);
    }
}

contract CounterTest is Test {
    RiscZeroMockVerifier private verifier;
    ERC20 private token;
    Counter private counter;

    function setUp() public {
        // fork from the actual Mainnet to get realistic results
        vm.createSelectFork(vm.rpcUrl("mainnet"));

        verifier = new RiscZeroMockVerifier(bytes4(0xFFFFFFFF));
        token = new ERC20FixedSupply("TOYKEN", "TOY", address(0x01));
        counter = new Counter(verifier, address(token));
    }

    function testIncrement() public {
        // get the hash of the previous block
        uint240 blockNumber = uint240(block.number - 1);
        bytes32 blockHash = blockhash(blockNumber);

        // mock the Journal
        Counter.Journal memory journal = Counter.Journal({
            commitment: Steel.Commitment(Encoding.encodeVersionedID(blockNumber, 0), blockHash, ChainSpec.configID()),
            tokenContract: address(token)
        });
        // create a mock proof
        RiscZeroReceipt memory receipt = verifier.mockProve(counter.imageId(), sha256(abi.encode(journal)));

        uint256 prevCount = counter.count();
        counter.increment(abi.encode(journal), receipt.seal);

        // check that the counter was incremented
        assert(counter.count() == prevCount + 1);
    }
}
