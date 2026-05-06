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
//! [`OpSpecId`] is a thin newtype around [`OpRevmSpecId`] (re-exported from op-revm) used
//! as Steel's [`EvmFactory::SpecId`](risc0_steel::EvmFactory::SpecId). The wrapper exists
//! only because the orphan rule blocks `impl EvmSpecId for op_revm::OpSpecId` from this
//! crate; all variant data still lives upstream.

pub use op_revm::spec::OpSpecId as OpRevmSpecId;
use risc0_steel::{EvmSpecId, revm::primitives::hardfork::SpecId};
use std::fmt;

/// [EvmFactory::SpecId](risc0_steel::EvmFactory::SpecId) for Optimism-family chains.
///
/// Thin newtype around [`OpRevmSpecId`]; convertible in both directions via [`From`].
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct OpSpecId(OpRevmSpecId);

impl OpSpecId {
    /// Bedrock.
    pub const BEDROCK: Self = Self(OpRevmSpecId::BEDROCK);
    /// Regolith.
    pub const REGOLITH: Self = Self(OpRevmSpecId::REGOLITH);
    /// Canyon.
    pub const CANYON: Self = Self(OpRevmSpecId::CANYON);
    /// Ecotone.
    pub const ECOTONE: Self = Self(OpRevmSpecId::ECOTONE);
    /// Fjord.
    pub const FJORD: Self = Self(OpRevmSpecId::FJORD);
    /// Granite.
    pub const GRANITE: Self = Self(OpRevmSpecId::GRANITE);
    /// Holocene.
    pub const HOLOCENE: Self = Self(OpRevmSpecId::HOLOCENE);
    /// Isthmus.
    pub const ISTHMUS: Self = Self(OpRevmSpecId::ISTHMUS);
    /// Jovian.
    pub const JOVIAN: Self = Self(OpRevmSpecId::JOVIAN);
    /// Karst (Osaka EVM + EIP-7823/7883 MODEXP + EIP-7951 P256VERIFY). Base names this
    /// hardfork "Azul"; see [`base::AZUL`] for that alias.
    pub const KARST: Self = Self(OpRevmSpecId::KARST);
    /// Interop.
    pub const INTEROP: Self = Self(OpRevmSpecId::INTEROP);

    /// Returns the underlying [`OpRevmSpecId`].
    #[inline]
    pub const fn into_inner(self) -> OpRevmSpecId {
        self.0
    }

    /// Converts into the corresponding revm [`SpecId`].
    #[inline]
    pub const fn into_eth_spec(self) -> SpecId {
        self.0.into_eth_spec()
    }
}

impl fmt::Display for OpSpecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(<&'static str>::from(self.0))
    }
}

impl From<OpRevmSpecId> for OpSpecId {
    #[inline]
    fn from(spec: OpRevmSpecId) -> Self {
        Self(spec)
    }
}

impl From<OpSpecId> for OpRevmSpecId {
    #[inline]
    fn from(spec: OpSpecId) -> Self {
        spec.0
    }
}

impl EvmSpecId for OpSpecId {
    #[inline]
    fn has_eip4788(&self) -> bool {
        self.0 >= OpRevmSpecId::ECOTONE
    }
    #[inline]
    fn has_eip2935(&self) -> bool {
        self.0 >= OpRevmSpecId::ISTHMUS
    }
    #[inline]
    fn to_u32(&self) -> u32 {
        self.0 as u32
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
    fn display_uses_op_revm_names() {
        assert_eq!(OpSpecId::KARST.to_string(), "Karst");
    }
}
