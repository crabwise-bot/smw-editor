use std::{convert::TryFrom, fmt, ops::*};

use duplicate::*;
use paste::*;
use thiserror::Error;

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AddressError {
    #[error("Invalid PC LoROM address {0:#x}")]
    InvalidPcLoRom(AddrPc),
    #[error("Invalid PC HiROM address {0:#x}")]
    InvalidPcHiRom(AddrPc),
    #[error("Invalid PC ExLoROM address {0:#x}")]
    InvalidPcExLoRom(AddrPc),
    #[error("Invalid PC ExHiROM address {0:#x}")]
    InvalidPcExHiRom(AddrPc),
    #[error("Invalid SNES LoROM address {0:#x}")]
    InvalidSnesLoRom(AddrSnes),
    #[error("Invalid SNES HiROM address {0:#x}")]
    InvalidSnesHiRom(AddrSnes),
    #[error("Invalid SNES ExLoROM address {0:#x}")]
    InvalidSnesExLoRom(AddrSnes),
    #[error("Invalid SNES ExHiROM address {0:#x}")]
    InvalidSnesExHiRom(AddrSnes),
}

// -------------------------------------------------------------------------------------------------

/// Bank
pub const MASK_BB: u32 = 0xFF0000;
/// High byte
pub const MASK_HH: u32 = 0x00FF00;
/// Low byte
pub const MASK_DD: u32 = 0x0000FF;
/// Absolute address
pub const MASK_HHDD: u32 = MASK_HH | MASK_DD;
/// Long address
pub const MASK_BBHHDD: u32 = MASK_BB | MASK_HH | MASK_DD;

// -------------------------------------------------------------------------------------------------

/// The arithmetic and ordering `RomSlice` needs from an address type. Deliberately narrow: the
/// address newtypes are offsets into a ROM, not general-purpose integers.
pub trait Addr:
    Copy + Ord + fmt::LowerHex + fmt::UpperHex + Add<usize, Output = Self> + Sub<usize, Output = Self>
{
    const MIN: Self;
}

// -------------------------------------------------------------------------------------------------

duplicate! {
    [
        addr_type   inner   fmt_lower_hex   fmt_upper_hex;
        [AddrPc]    [u32]   ["PC {:#x}"]    ["PC {:#X}"];
        [AddrSnes]  [u32]   ["SNES ${:x}"]  ["SNES ${:X}"];
        [AddrVram]  [u16]   ["VRAM ${:x}"]  ["VRAM ${:X}"];
    ]

    #[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
    pub struct addr_type(pub inner);

    impl addr_type {
        #[inline]
        pub fn as_index(self) -> usize {
            self.0 as usize
        }
    }

    impl From<addr_type> for inner {
        #[inline]
        fn from(addr: addr_type) -> Self {
            addr.0
        }
    }

    impl Default for addr_type {
        #[inline]
        fn default() -> Self {
            Self::MIN
        }
    }

    impl fmt::LowerHex for addr_type {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, fmt_lower_hex, self.0)
        }
    }

    impl fmt::UpperHex for addr_type {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, fmt_upper_hex, self.0)
        }
    }

    #[duplicate_item(
        op_name     op;
        [Add]       [+];
        [Sub]       [-];
        [Mul]       [*];
        [Div]       [/];
        [Rem]       [%];
        [BitAnd]    [&];
        [BitOr]     [|];
        [BitXor]    [^];
        [Shl]       [<<];
        [Shr]       [>>];
    )]
    paste! {
        impl<I: TryInto<inner>> op_name<I> for addr_type {
            type Output = Self;
            fn [<op_name:lower>](self, rhs: I) -> Self::Output {
                Self(self.0 op rhs.try_into().ok().expect("address operand out of range"))
            }
        }
        impl<I: TryInto<inner>> [<op_name Assign>]<I> for addr_type {
            fn [<op_name:lower _assign>](&mut self, rhs: I) {
                self.0 = self.0 op rhs.try_into().ok().expect("address operand out of range");
            }
        }
    }

}

impl From<AddrVram> for u32 {
    #[inline]
    fn from(addr: AddrVram) -> Self {
        addr.0 as u32
    }
}

impl fmt::Debug for AddrVram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AddrVram({:#06x})", self.0)
    }
}

duplicate! {
    [
        addr_type   opposite_type;
        [AddrPc]    [AddrSnes];
        [AddrSnes]  [AddrPc];
    ]
    impl fmt::Debug for addr_type {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match opposite_type::try_from(*self) {
                Ok(opposite) => write!(f, "{}({:#06X}) [-> {opposite:X}]", stringify!(addr_type), self.0),
                Err(_) => write!(f, "{}({:#06X})", stringify!(addr_type), self.0),
            }
        }
    }

    impl fmt::Display for addr_type {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match opposite_type::try_from(*self) {
                Ok(opposite) => write!(f, "{:#06X} [-> {opposite:X}]", self.0),
                Err(_) => write!(f, "{:#06X}", self.0),
            }
        }
    }

    impl TryFrom<opposite_type> for addr_type {
        type Error = AddressError;

        fn try_from(value: opposite_type) -> Result<Self, Self::Error> {
            Self::try_from_lorom(value)
        }
    }
}

// -------------------------------------------------------------------------------------------------

impl Addr for AddrPc {
    const MIN: Self = AddrPc(0);
}

impl AddrPc {
    pub fn try_from_lorom(addr: AddrSnes) -> Result<Self, AddressError> {
        if addr.is_valid_lorom() {
            Ok(Self(((addr.0 & 0x7F0000) >> 1) | (addr.0 & 0x7FFF)))
        } else {
            Err(AddressError::InvalidSnesLoRom(addr))
        }
    }

    pub fn try_from_hirom(addr: AddrSnes) -> Result<Self, AddressError> {
        if addr.is_valid_hirom() {
            Ok(Self(addr.0 & 0x3FFFFF))
        } else {
            Err(AddressError::InvalidSnesHiRom(addr))
        }
    }

    /// ExLoROM: like LoROM for banks $00-$7D, plus a second 4 MiB window at
    /// banks $80-$FF:$8000-$FFFF (where LoROM would mirror the first 4 MiB).
    pub fn try_from_exlorom(addr: AddrSnes) -> Result<Self, AddressError> {
        if addr.is_valid_exlorom() {
            let bank = (addr.0 >> 16) & 0xFF;
            Ok(Self(if bank >= 0x80 {
                0x400000 | (((addr.0 & 0x7F0000) >> 1) | (addr.0 & 0x7FFF))
            } else {
                ((addr.0 & 0x7F0000) >> 1) | (addr.0 & 0x7FFF)
            }))
        } else {
            Err(AddressError::InvalidSnesExLoRom(addr))
        }
    }

    /// ExHiROM: like HiROM for banks $40-$7D/$C0-$FF, plus a second 4 MiB
    /// window at banks $80-$BF:$0000-$FFFF (where HiROM would mirror).
    pub fn try_from_exhirom(addr: AddrSnes) -> Result<Self, AddressError> {
        if addr.is_valid_exhirom() {
            let bank = (addr.0 >> 16) & 0xFF;
            Ok(Self(if (0x80..=0xBF).contains(&bank) { 0x400000 | (addr.0 & 0x3FFFFF) } else { addr.0 & 0x3FFFFF }))
        } else {
            Err(AddressError::InvalidSnesExHiRom(addr))
        }
    }

    pub fn is_valid_lorom(&self) -> bool {
        self.0 < 0x400000
    }

    pub fn is_valid_hirom(&self) -> bool {
        self.0 < 0x400000
    }

    pub fn is_valid_exlorom(&self) -> bool {
        self.0 < 0x800000
    }

    pub fn is_valid_exhirom(&self) -> bool {
        self.0 < 0x800000
    }
}

impl Addr for AddrSnes {
    const MIN: Self = AddrSnes(0x8000);
}

impl AddrSnes {
    pub fn try_from_lorom(addr: AddrPc) -> Result<Self, AddressError> {
        if addr.is_valid_lorom() {
            Ok(Self(((addr.0 << 1) & 0x7F0000) | (addr.0 & 0x7FFF) | 0x8000))
        } else {
            Err(AddressError::InvalidPcLoRom(addr))
        }
    }

    pub fn try_from_hirom(addr: AddrPc) -> Result<Self, AddressError> {
        if addr.is_valid_hirom() {
            Ok(Self(addr.0 | 0xC00000))
        } else {
            Err(AddressError::InvalidPcHiRom(addr))
        }
    }

    pub fn try_from_exlorom(addr: AddrPc) -> Result<Self, AddressError> {
        if addr.is_valid_exlorom() {
            Ok(Self(if addr.0 < 0x400000 {
                ((addr.0 << 1) & 0x7F0000) | (addr.0 & 0x7FFF) | 0x8000
            } else {
                let bank = 0x80 | ((addr.0 >> 15) & 0x7F);
                (bank << 16) | 0x8000 | (addr.0 & 0x7FFF)
            }))
        } else {
            Err(AddressError::InvalidPcExLoRom(addr))
        }
    }

    pub fn try_from_exhirom(addr: AddrPc) -> Result<Self, AddressError> {
        if addr.is_valid_exhirom() {
            Ok(Self(if addr.0 < 0x400000 { addr.0 | 0xC00000 } else { 0x800000 | (addr.0 & 0x3FFFFF) }))
        } else {
            Err(AddressError::InvalidPcExHiRom(addr))
        }
    }

    pub fn is_valid_lorom(&self) -> bool {
        let wram = (self.0 & 0xFE0000) == 0x7E0000;
        let junk = (self.0 & 0x408000) == 0x000000;
        let sram = (self.0 & 0x708000) == 0x700000;
        !wram && !junk && !sram
    }

    pub fn is_valid_hirom(&self) -> bool {
        let wram = (self.0 & 0xFE0000) == 0x7E0000;
        let junk = (self.0 & 0x408000) == 0x000000;
        !wram && !junk
    }

    pub fn is_valid_exlorom(&self) -> bool {
        let bank = (self.0 >> 16) & 0xFF;
        // Upper 4 MiB window: banks $80-$FF, upper half of each bank.
        ((0x80..=0xFF).contains(&bank) && (self.0 & 0x8000) != 0) || self.is_valid_lorom()
    }

    pub fn is_valid_exhirom(&self) -> bool {
        let bank = (self.0 >> 16) & 0xFF;
        // Upper 4 MiB window: banks $80-$BF, whole banks.
        (0x80..=0xBF).contains(&bank) || self.is_valid_hirom()
    }
}

impl AddrSnes {
    #[must_use]
    pub fn bank(self) -> u8 {
        (self.0 >> 16) as u8
    }

    #[must_use]
    pub fn high(self) -> u8 {
        ((self.0 & MASK_HH) >> 8) as u8
    }

    #[must_use]
    pub fn low(self) -> u8 {
        (self.0 & MASK_DD) as u8
    }

    #[must_use]
    pub fn absolute(self) -> u16 {
        (self.0 & MASK_HHDD) as u16
    }

    #[must_use]
    pub fn with_bank(self, bank: u8) -> Self {
        Self((self.0 & 0x00FFFF) | ((bank as u32) << 16))
    }

    #[must_use]
    pub fn with_high(self, high: u8) -> Self {
        Self((self.0 & 0xFF00FF) | ((high as u32) << 8))
    }

    #[must_use]
    pub fn with_low(self, low: u8) -> Self {
        Self((self.0 & 0xFFFF00) | (low as u32))
    }

    #[must_use]
    pub fn with_absolute(self, absolute: u16) -> Self {
        Self((self.0 & 0xFF0000) | (absolute as u32))
    }
}

impl Addr for AddrVram {
    const MIN: Self = Self(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exlorom_snes_to_pc_covers_both_windows() {
        // Lower 4 MiB behaves like LoROM.
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0x008000)).unwrap(), AddrPc(0x000000));
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0x0FFFFF)).unwrap(), AddrPc(0x07FFFF));
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0x7DFFFF)).unwrap(), AddrPc(0x3EFFFF));
        // Upper 4 MiB window: banks $80-$FF:$8000-$FFFF (no LoROM mirroring).
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0x808000)).unwrap(), AddrPc(0x400000));
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0xBFFFFF)).unwrap(), AddrPc(0x5FFFFF));
        assert_eq!(AddrPc::try_from_exlorom(AddrSnes(0xFFFFFF)).unwrap(), AddrPc(0x7FFFFF));
    }

    #[test]
    fn exlorom_pc_to_snes_covers_both_windows() {
        assert_eq!(AddrSnes::try_from_exlorom(AddrPc(0x000000)).unwrap(), AddrSnes(0x008000));
        assert_eq!(AddrSnes::try_from_exlorom(AddrPc(0x3EFFFF)).unwrap(), AddrSnes(0x7DFFFF));
        assert_eq!(AddrSnes::try_from_exlorom(AddrPc(0x400000)).unwrap(), AddrSnes(0x808000));
        assert_eq!(AddrSnes::try_from_exlorom(AddrPc(0x7FFFFF)).unwrap(), AddrSnes(0xFFFFFF));
        // Round trips.
        for pc in [0x000000u32, 0x123456, 0x3EFFFF, 0x400000, 0x5ABCDE, 0x7FFFFF] {
            let snes = AddrSnes::try_from_exlorom(AddrPc(pc)).unwrap();
            assert_eq!(AddrPc::try_from_exlorom(snes).unwrap(), AddrPc(pc), "pc={pc:#x}");
        }
    }

    #[test]
    fn exlorom_rejects_bad_addresses() {
        // WRAM, hardware/junk mirrors, SRAM, and anything past 8 MiB are rejected.
        // ($70:$8000 is a valid LoROM ROM mirror, not SRAM, so it stays accepted.)
        for bad in [0x7E8000u32, 0x001234, 0x700000, 0x801234, 0x7E0000] {
            assert!(AddrPc::try_from_exlorom(AddrSnes(bad)).is_err(), "snes={bad:#x}");
        }
        assert!(AddrSnes::try_from_exlorom(AddrPc(0x800000)).is_err());
    }

    #[test]
    fn exhirom_snes_to_pc_covers_both_windows() {
        // Lower 4 MiB behaves like HiROM.
        assert_eq!(AddrPc::try_from_exhirom(AddrSnes(0xC00000)).unwrap(), AddrPc(0x000000));
        assert_eq!(AddrPc::try_from_exhirom(AddrSnes(0xFFFFFF)).unwrap(), AddrPc(0x3FFFFF));
        // Upper 4 MiB window: banks $80-$BF, whole banks.
        assert_eq!(AddrPc::try_from_exhirom(AddrSnes(0x800000)).unwrap(), AddrPc(0x400000));
        assert_eq!(AddrPc::try_from_exhirom(AddrSnes(0xBFFFFF)).unwrap(), AddrPc(0x7FFFFF));
        assert_eq!(AddrPc::try_from_exhirom(AddrSnes(0x9ABCDE)).unwrap(), AddrPc(0x5ABCDE));
    }

    #[test]
    fn exhirom_pc_to_snes_covers_both_windows() {
        assert_eq!(AddrSnes::try_from_exhirom(AddrPc(0x000000)).unwrap(), AddrSnes(0xC00000));
        assert_eq!(AddrSnes::try_from_exhirom(AddrPc(0x3FFFFF)).unwrap(), AddrSnes(0xFFFFFF));
        assert_eq!(AddrSnes::try_from_exhirom(AddrPc(0x400000)).unwrap(), AddrSnes(0x800000));
        assert_eq!(AddrSnes::try_from_exhirom(AddrPc(0x7FFFFF)).unwrap(), AddrSnes(0xBFFFFF));
        for pc in [0x000000u32, 0x123456, 0x3FFFFF, 0x400000, 0x5ABCDE, 0x7FFFFF] {
            let snes = AddrSnes::try_from_exhirom(AddrPc(pc)).unwrap();
            assert_eq!(AddrPc::try_from_exhirom(snes).unwrap(), AddrPc(pc), "pc={pc:#x}");
        }
    }

    #[test]
    fn exhirom_rejects_bad_addresses() {
        for bad in [0x7E0000u32, 0x001234, 0x7FFFFF] {
            assert!(AddrPc::try_from_exhirom(AddrSnes(bad)).is_err(), "snes={bad:#x}");
        }
        assert!(AddrSnes::try_from_exhirom(AddrPc(0x800000)).is_err());
    }

    #[test]
    fn lorom_hirom_basics_unchanged() {
        // Spot-check the pre-existing conversions still behave.
        assert_eq!(AddrPc::try_from_lorom(AddrSnes(0x008000)).unwrap(), AddrPc(0x000000));
        assert_eq!(AddrSnes::try_from_lorom(AddrPc(0x000000)).unwrap(), AddrSnes(0x008000));
        assert_eq!(AddrPc::try_from_hirom(AddrSnes(0xC00000)).unwrap(), AddrPc(0x000000));
        assert_eq!(AddrSnes::try_from_hirom(AddrPc(0x000000)).unwrap(), AddrSnes(0xC00000));
    }
}
