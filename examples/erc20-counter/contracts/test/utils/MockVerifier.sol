// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.26;

import {RiscZeroMockVerifier} from "risc0-ethereum/test/RiscZeroMockVerifier.sol";

contract MockVerifier is RiscZeroMockVerifier(bytes4(0xFFFFFFFF)) {}
