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

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../contracts/src/Counter.sol");
    println!("cargo:rerun-if-changed=../contracts/test/utils/MockVerifier.sol");
    println!("cargo:rerun-if-env-changed=SKIP_FORGE_BUILD");

    // Allow explicitly skipping the forge build (useful for CI or when contracts are pre-built)
    if env::var("SKIP_FORGE_BUILD").is_ok() {
        println!("cargo:warning=SKIP_FORGE_BUILD is set, skipping contract build");
        return;
    }

    if Command::new("forge").arg("--version").status().is_err() {
        panic!("Forge is required. Install it or set SKIP_FORGE_BUILD=1 to skip.");
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // host/ -> examples/erc20-counter/
    let foundry_root = manifest_dir.parent().unwrap();

    // Run forge build, only build exactly what we need for tests to save time
    let status = Command::new("forge")
        .env("FOUNDRY_LINT_LINT_ON_BUILD", "false")
        .args(["build", "--root"])
        .arg(foundry_root)
        .arg("contracts/src/Counter.sol")
        .arg("contracts/test/utils/MockVerifier.sol")
        .status()
        .expect("Failed to execute forge");
    assert!(status.success(), "Forge build failed");
}
