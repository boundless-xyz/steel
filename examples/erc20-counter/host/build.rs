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
    // Gracefully skip if Forge is not installed.
    // This allows the crate to compile in CI environments that don't need the artifacts.
    if Command::new("forge").arg("--version").status().is_err() {
        println!("cargo:warning=Forge not found, skipping contract build");
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // host/ -> examples/erc20-counter/
    let foundry_root = manifest_dir.parent().unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../contracts/src/Counter.sol");
    println!("cargo:rerun-if-changed=../contracts/test/utils/MockVerifier.sol");

    // Run Forge Build.
    let status = Command::new("forge")
        .env("FOUNDRY_LINT_LINT_ON_BUILD", "false")
        .args(["build", "--root"])
        .arg(foundry_root)
        // Only build exactly what we need for tests to save time
        .arg("contracts/src/Counter.sol")
        .arg("contracts/test/utils/MockVerifier.sol")
        .status()
        .expect("Failed to execute forge");
    assert!(status.success(), "Forge build failed");
}
