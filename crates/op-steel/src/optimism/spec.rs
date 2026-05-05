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
//! Defines [`OpSpecId`], the Optimism/Base hardfork spec used by Steel. The enum mirrors
//! op-revm's [`OpRevmSpecId`] one-to-one — it exists locally only so [`EvmSpecId`] can be
//! implemented on it (orphan rule blocks the impl on `op_revm::OpSpecId` from this crate).

use op_revm::spec::{OpSpecId as OpRevmSpecId, name};
use risc0_steel::{EvmSpecId, revm::primitives::hardfork::SpecId};
use std::fmt;

/// [EvmFactory::SpecId](risc0_steel::EvmFactory::SpecId) for Optimism-family chains.
///
/// Mirrors [`OpRevmSpecId`] from op-revm 20: variants and discriminants are identical, and
/// [`From<OpSpecId>`] for both [`OpRevmSpecId`] and [`SpecId`] are pure passthroughs to the
/// upstream conversions. Steel maintains its own copy only because [`EvmSpecId`] is defined
/// here and the orphan rule blocks implementing it directly on [`OpRevmSpecId`].
///
/// Base's "Azul" hardfork is the same as upstream `Karst` at the EVM/precompile level
/// (Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951 P256VERIFY); see [`base::AZUL`] for a
/// chain-spec-friendly alias.
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
    /// Karst (Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951 P256VERIFY). Base names this
    /// hardfork "Azul"; see [`base::AZUL`] for that alias.
    KARST,
    /// Interop.
    INTEROP,
}

impl OpSpecId {
    /// Converts into the corresponding revm [`SpecId`].
    #[inline]
    pub const fn into_eth_spec(self) -> SpecId {
        to_op_revm(self).into_eth_spec()
    }
}

#[inline]
const fn to_op_revm(spec: OpSpecId) -> OpRevmSpecId {
    match spec {
        OpSpecId::INTEROP => OpRevmSpecId::INTEROP,
        OpSpecId::KARST => OpRevmSpecId::KARST,
        OpSpecId::JOVIAN => OpRevmSpecId::JOVIAN,
        OpSpecId::ISTHMUS => OpRevmSpecId::ISTHMUS,
        OpSpecId::HOLOCENE => OpRevmSpecId::HOLOCENE,
        OpSpecId::GRANITE => OpRevmSpecId::GRANITE,
        OpSpecId::FJORD => OpRevmSpecId::FJORD,
        OpSpecId::ECOTONE => OpRevmSpecId::ECOTONE,
        OpSpecId::CANYON => OpRevmSpecId::CANYON,
        OpSpecId::REGOLITH => OpRevmSpecId::REGOLITH,
        OpSpecId::BEDROCK => OpRevmSpecId::BEDROCK,
    }
}

impl From<OpSpecId> for OpRevmSpecId {
    #[inline]
    fn from(spec: OpSpecId) -> Self {
        to_op_revm(spec)
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
            Self::KARST => name::KARST,
            Self::INTEROP => name::INTEROP,
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

/// Base-specific aliases. Base names its Karst-equivalent hardfork "Azul"; this module
/// exposes that name as a constant so chain-spec literals read naturally without
/// introducing a parallel `BaseSpecId` enum.
pub mod base {
    use super::OpSpecId;

    /// Base's name for the Karst-equivalent EVM/precompile fork. EVM-level behavior is
    /// identical to [`OpSpecId::KARST`]: Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951
    /// P256VERIFY. Used by Base Sepolia / Base Mainnet chain specs.
    pub const AZUL: OpSpecId = OpSpecId::KARST;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn karst_maps_to_osaka() {
        assert_eq!(OpSpecId::KARST.into_eth_spec(), SpecId::OSAKA);
        assert_eq!(OpRevmSpecId::from(OpSpecId::KARST), OpRevmSpecId::KARST);
    }

    #[test]
    fn base_azul_aliases_karst() {
        assert_eq!(base::AZUL, OpSpecId::KARST);
    }

    #[test]
    fn discriminants_match_op_revm() {
        // Steel's enum exists only because of the orphan rule; its discriminants must
        // stay aligned with op-revm so chain-spec digests aren't tied to a Steel-local
        // numbering that could drift.
        assert_eq!(OpSpecId::BEDROCK as u8, OpRevmSpecId::BEDROCK as u8);
        assert_eq!(OpSpecId::JOVIAN as u8, OpRevmSpecId::JOVIAN as u8);
        assert_eq!(OpSpecId::KARST as u8, OpRevmSpecId::KARST as u8);
        assert_eq!(OpSpecId::INTEROP as u8, OpRevmSpecId::INTEROP as u8);
    }
}
