// SPDX-License-Identifier: Apache-2.0

pragma solidity ^0.8.26;

import {ERC20} from "openzeppelin/contracts/token/ERC20/ERC20.sol";

contract FixedSupplyToken is ERC20 {
    constructor(string memory name, string memory symbol, address owner) ERC20(name, symbol) {
        _mint(owner, 1000);
    }
}
