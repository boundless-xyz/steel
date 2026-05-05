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

//! Spec-id plumbing for Optimism-family chains.
//!
//! Defines [`OpSpecId`], the Optimism/Base hardfork spec used by Steel, along with its
//! conversion into [`OpRevmSpecId`] (op-revm) and [`SpecId`] (revm), and the Azul
//! precompile overlay applied by the factory when `AZUL` is active.

use op_revm::{
    precompiles as op_precompiles,
    spec::{OpSpecId as OpRevmSpecId, name},
};
use risc0_steel::{
    EvmSpecId,
    revm::{
        precompile::{Precompiles, modexp, secp256r1},
        primitives::hardfork::SpecId,
    },
};
use std::{fmt, sync::OnceLock};

/// [EvmFactory::SpecId](risc0_steel::EvmFactory::SpecId) for Optimism-family chains.
///
/// Mirrors op-revm's [`OpRevmSpecId`] with one extra variant: [`OpSpecId::AZUL`], a
/// Base-specific fork that activates Osaka EVM semantics plus the MODEXP (EIP-7823/7883) and
/// P256VERIFY (EIP-7951) precompile upgrades. Base treats Azul as a distinct hardfork name
/// rather than reusing `Karst`, and Steel follows suit so chain-spec declarations read
/// naturally and the factory dispatches precompiles off the spec itself rather than chain-id
/// heuristics.
///
/// The upstream `KARST` spec (op-revm's Osaka-equivalent fork name) is intentionally omitted:
/// OP Stack hasn't ratified Karst for any chain Steel runs against, so the upstream variant
/// has no well-defined semantics here. Base ships Osaka's EL changes as `AZUL` (with extra
/// precompile rules); OP chains have not declared one yet.
///
/// Discriminant values match [`OpRevmSpecId`] for every shared variant up to `JOVIAN` (108).
/// `INTEROP` keeps Steel's original 109 (op-revm 20 shifted it to 110 when it inserted KARST
/// at 109), and `AZUL` is pinned at 111 so [`ChainSpec`](risc0_steel::config::ChainSpec)
/// digests for OP and Base chains are unchanged across the op-revm 17→20 bump.
#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
pub enum OpSpecId {
    /// Bedrock.
    BEDROCK = 100,
    /// Regolith.
    REGOLITH,
    /// Canyon.
    CANYON,
    /// Ecotone.
    ECOTONE,
    /// Fjord.
    FJORD,
    /// Granite.
    GRANITE,
    /// Holocene.
    HOLOCENE,
    /// Isthmus.
    ISTHMUS,
    /// Jovian.
    #[default]
    JOVIAN,
    /// Interop.
    INTEROP,
    /// Azul (Base). Osaka EVM + Base-specific MODEXP / P256VERIFY precompile overlay.
    /// Discriminant skips 110 (which would be upstream `OSAKA`) so Base Sepolia digests are
    /// stable if an `OSAKA` variant is ever reintroduced.
    AZUL = 111,
}

impl OpSpecId {
    /// Converts into the corresponding revm [`SpecId`]. `AZUL` maps to `OSAKA` since Azul's
    /// opcode-level semantics are identical to Osaka.
    #[inline]
    pub const fn into_eth_spec(self) -> SpecId {
        match self {
            Self::AZUL => SpecId::OSAKA,
            Self::ISTHMUS | Self::JOVIAN | Self::INTEROP => SpecId::PRAGUE,
            Self::ECOTONE | Self::FJORD | Self::GRANITE | Self::HOLOCENE => SpecId::CANCUN,
            Self::CANYON => SpecId::SHANGHAI,
            Self::BEDROCK | Self::REGOLITH => SpecId::MERGE,
        }
    }
}

/// Conversion into op-revm's [`OpRevmSpecId`] for revm-level dispatch. `AZUL` maps to
/// [`OpRevmSpecId::KARST`] — Azul's opcode-level semantics match Karst (op-revm 20's rename
/// of the former `OSAKA` variant); the Base-specific precompile overlay is applied separately
/// by the factory, which branches on `OpSpecId::AZUL` before the conversion happens.
impl From<OpSpecId> for OpRevmSpecId {
    fn from(spec: OpSpecId) -> Self {
        match spec {
            OpSpecId::AZUL => Self::KARST,
            OpSpecId::INTEROP => Self::INTEROP,
            OpSpecId::JOVIAN => Self::JOVIAN,
            OpSpecId::ISTHMUS => Self::ISTHMUS,
            OpSpecId::HOLOCENE => Self::HOLOCENE,
            OpSpecId::GRANITE => Self::GRANITE,
            OpSpecId::FJORD => Self::FJORD,
            OpSpecId::ECOTONE => Self::ECOTONE,
            OpSpecId::CANYON => Self::CANYON,
            OpSpecId::REGOLITH => Self::REGOLITH,
            OpSpecId::BEDROCK => Self::BEDROCK,
        }
    }
}

impl fmt::Display for OpSpecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BEDROCK => name::BEDROCK,
            Self::REGOLITH => name::REGOLITH,
            Self::CANYON => name::CANYON,
            Self::ECOTONE => name::ECOTONE,
            Self::FJORD => name::FJORD,
            Self::GRANITE => name::GRANITE,
            Self::HOLOCENE => name::HOLOCENE,
            Self::ISTHMUS => name::ISTHMUS,
            Self::JOVIAN => name::JOVIAN,
            Self::INTEROP => name::INTEROP,
            Self::AZUL => "Azul",
        })
    }
}

impl EvmSpecId for OpSpecId {
    #[inline]
    fn has_eip4788(&self) -> bool {
        *self >= Self::ECOTONE
    }
    #[inline]
    fn has_eip2935(&self) -> bool {
        *self >= Self::ISTHMUS
    }
    #[inline]
    fn to_u32(&self) -> u32 {
        *self as u32
    }
}

/// Precompile set for the Base Azul hardfork.
///
/// Mirrors `BasePrecompiles::azul()` from `base/base`: op-revm's `jovian()` set plus the
/// upstream `modexp::OSAKA` (EIP-7823 input cap + EIP-7883 gas) and `secp256r1::P256VERIFY_OSAKA`
/// (EIP-7951, 6,900 gas) precompiles. op-revm's own `OpRevmSpecId::KARST` still resolves to the
/// `jovian()` set as a placeholder, so Steel applies this overlay itself when `AZUL` is active.
pub(super) fn azul_precompiles() -> &'static Precompiles {
    static INSTANCE: OnceLock<Precompiles> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = op_precompiles::jovian().clone();
        precompiles.extend([modexp::OSAKA, secp256r1::P256VERIFY_OSAKA]);
        precompiles
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use risc0_steel::revm::precompile::{PrecompileHalt, bn254};

    fn encode_length(len: usize) -> [u8; 32] {
        let mut encoded = [0u8; 32];
        encoded[24..].copy_from_slice(&(len as u64).to_be_bytes());
        encoded
    }

    fn modexp_input(base_len: usize, exp_len: usize, mod_len: usize) -> Vec<u8> {
        let mut input = Vec::new();
        input.extend_from_slice(&encode_length(base_len));
        input.extend_from_slice(&encode_length(exp_len));
        input.extend_from_slice(&encode_length(mod_len));
        input.extend(vec![1u8; base_len + exp_len + mod_len]);
        input
    }

    #[test]
    fn azul_spec_conversion() {
        assert_eq!(OpSpecId::AZUL.into_eth_spec(), SpecId::OSAKA);
        assert_eq!(OpRevmSpecId::from(OpSpecId::AZUL), OpRevmSpecId::KARST);
    }

    #[test]
    fn overlay_installs_osaka_precompiles() {
        let p = azul_precompiles();

        // modexp at 0x05 is the Osaka variant (EIP-7823 rejects fields > 1024 bytes).
        let modexp = p.get(modexp::OSAKA.address()).unwrap();
        let out = modexp
            .execute(&modexp_input(1025, 0, 1), u64::MAX, 0)
            .unwrap();
        assert_eq!(
            out.halt_reason(),
            Some(&PrecompileHalt::ModexpEip7823LimitSize)
        );

        // p256verify at 0x100 is the Osaka variant (6,900 gas, not Fjord's 3,450).
        let p256 = p.get(secp256r1::P256VERIFY_OSAKA.address()).unwrap();
        let out = p256.execute(&[], 3_450, 0).unwrap();
        assert_eq!(out.halt_reason(), Some(&PrecompileHalt::OutOfGas));
    }

    #[test]
    fn overlay_preserves_jovian_bn254_pair_limit() {
        // The `extend()` in `azul_precompiles` must not clobber Jovian's bn254-pair override.
        let pair = azul_precompiles().get(&bn254::pair::ADDRESS).unwrap();
        let oversized = vec![0u8; op_precompiles::bn254_pair::JOVIAN_MAX_INPUT_SIZE + 1];
        let out = pair.execute(&oversized, u64::MAX, 0).unwrap();
        assert_eq!(out.halt_reason(), Some(&PrecompileHalt::Bn254PairLength));
    }
}
