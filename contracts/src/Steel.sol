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

pragma solidity ^0.8.9;

import {Blockhash} from "openzeppelin/contracts/utils/Blockhash.sol";

/// @title Steel Library
/// @notice This library provides a collection of utilities to work with Steel commitments in Solidity.
library Steel {
    /// @notice Represents a commitment to a specific block in the blockchain.
    /// @dev The `id` combines the version and the actual identifier of the claim, such as the block number.
    /// @dev The `digest` represents the data being committed to, e.g. the hash of the execution block.
    /// @dev The `configID` is the cryptographic digest of the network configuration.
    struct Commitment {
        uint256 id;
        bytes32 digest;
        bytes32 configID;
    }

    /// @notice The version of the Commitment not supported.
    /// @dev Error selector: 0x04e2dd22
    error InvalidCommitmentVersion(uint16 version);

    /// @notice The block number in the commitment is too old or invalid.
    /// @dev Error selector: 0xd7c29a3d
    error InvalidCommitmentBlockNumber();

    /// @notice The consensus slot (version 2) commitment is not supported.
    /// @dev Error selector: 0x13d71698
    error ConsensusSlotCommitmentNotSupported();

    /// @notice The config ID (i.e. chain spec digest) does not match the expected value.
    /// @dev Error selector: 0xa7e6de3e
    error InvalidConfigID(bytes32 expected, bytes32 received);

    /// @notice Validates if the provided commitment commits to a block in the current chain,
    ///         and contains the expected config ID for the current chain.
    /// @param commitment The Commitment struct to validate.
    /// @return True if the commitment commits to a block in the current chain, false otherwise.
    function validateCommitment(Commitment memory commitment) internal view returns (bool) {
        return validateCommitmentWithConfig(commitment, ChainSpec.configID());
    }

    /// @notice Validates if the provided commitment commits to a block in the current chain,
    ///         and contains the given config ID.
    /// @param commitment The Commitment struct to validate.
    /// @param configID The expected configID for the commitment. The configID commits to the chain
    ///        specification used to instantiate the EVM within Steel.
    /// @return True if the commitment commits to a block in the current chain, false otherwise.
    function validateCommitmentWithConfig(Commitment memory commitment, bytes32 configID)
        internal
        view
        returns (bool)
    {
        if (configID != commitment.configID) {
            revert InvalidConfigID(configID, commitment.configID);
        }
        (uint240 claimID, uint16 version) = Encoding.decodeVersionedID(commitment.id);
        if (version == 0) {
            return validateBlockCommitment(claimID, commitment.digest);
        } else if (version == 1) {
            return validateBeaconCommitment(claimID, commitment.digest);
        } else if (version == 2) {
            // Stateless SteelVerifier does not support consensus slot commitments.
            revert ConsensusSlotCommitmentNotSupported();
        } else {
            revert InvalidCommitmentVersion(version);
        }
    }

    /// @notice Validates if the provided block commitment matches the block hash of the given block number.
    /// @param blockNumber The block number to compare against.
    /// @param blockHash The block hash to validate.
    /// @return True if the block's block hash matches the block hash, false otherwise.
    function validateBlockCommitment(uint256 blockNumber, bytes32 blockHash) internal view returns (bool) {
        // NOTE: blockHash returns all zeroes if the block number is too far in the past.
        bytes32 blockHashResult = Blockhash.blockHash(blockNumber);
        if (blockHashResult == bytes32(0)) {
            revert InvalidCommitmentBlockNumber();
        }
        return blockHash == blockHashResult;
    }

    /// @notice Validates if the provided beacon commitment matches the block root of the given timestamp.
    /// @param timestamp The timestamp to compare against.
    /// @param blockRoot The block root to validate.
    /// @return True if the block's block root matches the block root, false otherwise.
    function validateBeaconCommitment(uint256 timestamp, bytes32 blockRoot) internal view returns (bool) {
        return blockRoot == Beacon.parentBlockRoot(timestamp);
    }
}

/// @title Beacon Library
library Beacon {
    /// @notice The address of the Beacon roots contract.
    /// @dev https://eips.ethereum.org/EIPS/eip-4788
    address internal constant BEACON_ROOTS_ADDRESS = 0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02;

    /// @notice Call to the EIP-4788 beacon roots contract failed due to an invalid block timestamp.
    /// @dev A block timestamp is invalid if it does not correspond to a stored block on the
    ///      EIP-4788 contract. This can happen if the timestamp is too old, and the corresponding
    ///      block has been evicted from the cache, if the timestamp corresponds to a
    ///      slot with no block, or if the timestamp does not correspond to any slot at all.
    /// @dev Error selector: 0x4d0b0a41
    error InvalidBlockTimestamp();

    /// @notice Find the root of the Beacon block corresponding to the parent of the execution block with the given timestamp.
    /// @return root Returns the corresponding Beacon block root or reverts, if no such block exists.
    function parentBlockRoot(uint256 timestamp) internal view returns (bytes32 root) {
        (bool success, bytes memory result) = BEACON_ROOTS_ADDRESS.staticcall(abi.encode(timestamp));
        if (success) {
            return abi.decode(result, (bytes32));
        } else {
            revert InvalidBlockTimestamp();
        }
    }
}

/// @title Encoding Library
library Encoding {
    /// @notice Encodes a version and ID into a single uint256 value.
    /// @param id The base ID to be encoded, limited by 240 bits (or the maximum value of a uint240).
    /// @param version The version number to be encoded, limited by 16 bits (or the maximum value of a uint16).
    /// @return Returns a single uint256 value that contains both the `id` and the `version` encoded into it.
    function encodeVersionedID(uint240 id, uint16 version) internal pure returns (uint256) {
        uint256 encoded;
        assembly {
            encoded := or(shl(240, version), id)
        }
        return encoded;
    }

    /// @notice Decodes a version and ID from a single uint256 value.
    /// @param id The single uint256 value to be decoded.
    /// @return Returns two values: a uint240 for the original base ID and a uint16 for the version number encoded into it.
    function decodeVersionedID(uint256 id) internal pure returns (uint240, uint16) {
        uint240 decoded;
        uint16 version;
        assembly {
            decoded := and(id, 0x0000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff)
            version := shr(240, id)
        }
        return (decoded, version);
    }
}

library ChainSpec {
    uint256 internal constant ETHEREUM_MAINNET_CHAIN_ID = 1;
    uint256 internal constant ETHEREUM_SEPOLIA_CHAIN_ID = 11155111;
    uint256 internal constant ETHEREUM_HOODI_CHAIN_ID = 560048;
    uint256 internal constant STEEL_TEST_OSAKA_CHAIN_ID = 5733100019;
    uint256 internal constant STEEL_TEST_PRAGUE_CHAIN_ID = 5733100018;

    /// @dev Error selector: 0x45b21e77
    error UnknownChainId(uint256 chainID);

    function configID() internal view returns (bytes32) {
        return configID(block.chainid);
    }

    // TODO(povw): Add something to keep this in sync with the Rust.
    function configID(uint256 chainID) internal pure returns (bytes32) {
        if (chainID == ETHEREUM_MAINNET_CHAIN_ID) {
            return hex"47dc59f84afd2e9e7a48c4012004ab7c77fbd9acf822bf1143b8442c6c8851d4";
        }
        if (chainID == ETHEREUM_SEPOLIA_CHAIN_ID) {
            return hex"90c1e882b1f0fda4dc7f1c66c07ed3d2a74e443834905faa9f32f583b71f459d";
        }
        if (chainID == ETHEREUM_HOODI_CHAIN_ID) {
            return hex"34cb1defd939572b00439d2c13f93c033b82227067371c910ad104d527c78860";
        }
        if (chainID == STEEL_TEST_OSAKA_CHAIN_ID) {
            return hex"2a80c688d324f578513161dda9e9a5773c0ee052f50304a94339e966da28b2ad";
        }
        if (chainID == STEEL_TEST_PRAGUE_CHAIN_ID) {
            return hex"33e32d9590cd4b168773ca27de65d535f2e744274b1437acb712dd4264f2eb87";
        }
        revert UnknownChainId(chainID);
    }
}
