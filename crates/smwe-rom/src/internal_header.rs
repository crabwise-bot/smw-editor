use std::{clone::Clone, convert::TryFrom, fmt};

use nom::{
    combinator::{map, map_res},
    multi::{count, many1},
    number::complete::{le_u16, le_u8},
    sequence::pair,
};
use num_enum::{IntoPrimitive, TryFromPrimitive};
use thiserror::Error;

use crate::{
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::Rom,
        rom_slice::PcSlice,
    },
    RomError,
};

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum InternalHeaderParseError {
    #[error("Couldn't find internal ROM header")]
    NotFound,
    #[error("Isolating Internal ROM Header:\n- {0}")]
    IsolatingData(RomError),

    #[error("Reading checksum and complement at LoROM location:\n- {0}")]
    ReadLoRomChecksum(RomError),
    #[error("Reading checksum and complement at HiROM location:\n- {0}")]
    ReadHiRomChecksum(RomError),
    #[error("Reading Internal ROM Name:\n- {0}")]
    ReadRomName(RomError),
    #[error("Reading Map Mode:\n- {0}")]
    ReadMapMode(RomError),
    #[error("Reading ROM Type:\n- {0}")]
    ReadRomType(RomError),
    #[error("Reading ROM Size:\n- {0}")]
    ReadRomSize(RomError),
    #[error("Reading SRAM Size:\n- {0}")]
    ReadSramSize(RomError),
    #[error("Reading Region Code:\n- {0}")]
    ReadRegionCode(RomError),
    #[error("Reading Developer ID:\n- {0}")]
    ReadDeveloperId(RomError),
    #[error("Reading Version Number:\n- {0}")]
    ReadVersionNumber(RomError),
    #[error("Reading Version Number:\n- {0}")]
    ReadNativeModeInterruptVectors(RomError),
    #[error("Reading Version Number:\n- {0}")]
    ReadEmulationModeInterruptVectors(RomError),
}

// -------------------------------------------------------------------------------------------------

#[rustfmt::skip]
pub mod offsets {
    pub const COMPLEMENT_CHECK: usize = 0x1C;
    pub const CHECKSUM:         usize = 0x1E;
}

#[rustfmt::skip]
pub mod sizes {
    pub const INTERNAL_HEADER:   usize = 64;
    pub const INTERNAL_ROM_NAME: usize = 21;
}

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct RomInternalHeader {
    pub internal_rom_name: String,
    pub map_mode:          MapMode,
    pub rom_type:          RomType,
    pub rom_size:          u8,
    pub sram_size:         u8,
    pub region_code:       RegionCode,
    pub developer_id:      u8,
    pub version_number:    u8,
    pub interrupt_vectors: Vec<AddrSnes>,
}

/// SNES cartridge memory map, from the internal header's map-mode byte.
///
/// The low nibble is Nintendo's "Mode 2x" number (Mode 20 = LoROM,
/// Mode 21 = HiROM, Mode 23/25 = SA-1 pack) with the community-standard
/// extensions Mode 22 = ExLoROM and Mode 24 = ExHiROM; bit 4 selects
/// FastROM. (Source: SNES Development Manual, "Map Mode (FFD5H)".)
#[derive(Copy, Clone, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum MapMode {
    SlowLoRom    = 0b100000,
    SlowHiRom    = 0b100001,
    SlowExLoRom  = 0b100010,
    SlowSa1LoRom = 0b100011,
    SlowExHiRom  = 0b100100,
    SlowSa1HiRom = 0b100101,
    FastLoRom    = 0b110000,
    FastHiRom    = 0b110001,
    FastExLoRom  = 0b110010,
    FastSa1LoRom = 0b110011,
    FastExHiRom  = 0b110100,
    FastSa1HiRom = 0b110101,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum RomType {
    Rom               = 0x00,
    RomRam            = 0x01,
    RomRamSram        = 0x02,

    RomDsp            = 0x03,
    RomSuperFx        = 0x13,
    RomObc1           = 0x23,
    RomSa1            = 0x33,
    RomSdd1           = 0x43,
    RomSrtc           = 0x53,
    RomOther          = 0xE3,
    RomCustom         = 0xF3,

    RomDspRam         = 0x04,
    RomSuperFxRam     = 0x14,
    RomObc1Ram        = 0x24,
    RomSa1Ram         = 0x34,
    RomSdd1Ram        = 0x44,
    RomSRtcRam        = 0x54,
    RomOtherRam       = 0xE4,
    RomCustomRam      = 0xF4,

    RomDspRamSram     = 0x05,
    RomSuperFxRamSram = 0x15,
    RomObc1RamSram    = 0x25,
    RomSa1RamSram     = 0x35,
    RomSdd1RamSram    = 0x45,
    RomSRtcRamSram    = 0x55,
    RomOtherRamSram   = 0xE5,
    RomCustomRamSram  = 0xF5,

    RomDspSram        = 0x06,
    RomSuperFxSram    = 0x16,
    RomObc1Sram       = 0x26,
    RomSa1Sram        = 0x36,
    RomSdd1Sram       = 0x46,
    RomSRtcSram       = 0x56,
    RomOtherSram      = 0xE6,
    RomCustomSram     = 0xF6,
}

#[derive(Debug, TryFromPrimitive)]
#[repr(u8)]
pub enum RegionCode {
    Japan        = 0x00,
    NorthAmerica = 0x01,
    Europe       = 0x02,
    Sweden       = 0x03,
    Finland      = 0x04,
    Denmark      = 0x05,
    France       = 0x06,
    Netherlands  = 0x07,
    Spain        = 0x08,
    Germany      = 0x09,
    Italy        = 0x0A,
    China        = 0x0B,
    Indonesia    = 0x0C,
    Korea        = 0x0D,
    Global       = 0x0E,
    Canada       = 0x0F,
    Brazil       = 0x10,
    Australia    = 0x11,
    Other1       = 0x12,
    Other2       = 0x13,
    Other3       = 0x14,
}

// -------------------------------------------------------------------------------------------------

impl RomInternalHeader {
    pub fn parse(rom: &Rom) -> Result<Self, InternalHeaderParseError> {
        let rih_slice = RomInternalHeader::find(rom)?;
        let name_slice = rih_slice.resize(sizes::INTERNAL_ROM_NAME);
        let byte_slice = name_slice.skip_forward(1).resize(1);

        Ok(Self {
            internal_rom_name: rom
                .parse_pc(name_slice, map_res(many1(le_u8), |s| std::str::from_utf8(&s).map(String::from)))
                .map_err(InternalHeaderParseError::ReadRomName)?,
            map_mode:          rom
                .parse_pc(byte_slice, map_res(le_u8, MapMode::try_from))
                .map_err(InternalHeaderParseError::ReadMapMode)?,
            rom_type:          rom
                .parse_pc(byte_slice.skip_forward(1), map_res(le_u8, RomType::try_from))
                .map_err(InternalHeaderParseError::ReadRomType)?,
            rom_size:          rom
                .parse_pc(byte_slice.skip_forward(2), le_u8)
                .map_err(InternalHeaderParseError::ReadRomSize)?,
            sram_size:         rom
                .parse_pc(byte_slice.skip_forward(3), le_u8)
                .map_err(InternalHeaderParseError::ReadSramSize)?,
            region_code:       rom
                .parse_pc(byte_slice.skip_forward(4), map_res(le_u8, RegionCode::try_from))
                .map_err(InternalHeaderParseError::ReadRegionCode)?,
            developer_id:      rom
                .parse_pc(byte_slice.skip_forward(5), le_u8)
                .map_err(InternalHeaderParseError::ReadDeveloperId)?,
            version_number:    rom
                .parse_pc(byte_slice.skip_forward(6), le_u8)
                .map_err(InternalHeaderParseError::ReadVersionNumber)?,
            interrupt_vectors: {
                let vectors_slice = byte_slice.skip_forward(15).resize(2 * 6);
                let mut parse_vectors = count(map(le_u16, |addr| AddrSnes(addr as _)), 6);
                let native = rom
                    .parse_pc(vectors_slice, &mut parse_vectors)
                    .map_err(InternalHeaderParseError::ReadNativeModeInterruptVectors)?;
                let emulation = rom
                    .parse_pc(vectors_slice.skip_forward(1).offset_forward(4), &mut parse_vectors)
                    .map_err(InternalHeaderParseError::ReadEmulationModeInterruptVectors)?;
                native.into_iter().chain(emulation).collect()
            },
        })
    }

    fn find(rom: &Rom) -> Result<PcSlice, InternalHeaderParseError> {
        const HEADER_LOROM: PcSlice = PcSlice::new(AddrPc(0x007FC0), sizes::INTERNAL_HEADER);
        const HEADER_HIROM: PcSlice = PcSlice::new(AddrPc(0x00FFC0), sizes::INTERNAL_HEADER);

        let lo_cpl_csm = HEADER_LOROM.offset_forward(offsets::COMPLEMENT_CHECK).resize(4);
        let hi_cpl_csm = HEADER_HIROM.offset_forward(offsets::COMPLEMENT_CHECK).resize(4);

        let (lo_cpl, lo_csm) =
            rom.parse_pc(lo_cpl_csm, pair(le_u16, le_u16)).map_err(InternalHeaderParseError::ReadLoRomChecksum)?;
        let (hi_cpl, hi_csm) =
            rom.parse_pc(hi_cpl_csm, pair(le_u16, le_u16)).map_err(InternalHeaderParseError::ReadHiRomChecksum)?;

        if (lo_csm ^ lo_cpl) == 0xFFFF {
            log::info!("Internal ROM header found at LoROM location: {:#X}", HEADER_LOROM.begin);
            Ok(HEADER_LOROM)
        } else if (hi_csm ^ hi_cpl) == 0xFFFF {
            log::info!("Internal ROM header found at HiROM location: {:#X}", HEADER_HIROM.begin);
            Ok(HEADER_HIROM)
        } else {
            log::error!("Couldn't find internal ROM header due to invalid checksums");
            log::error!("(LoROM: {:X}^{:X}, HiROM: {:X}^{:X})", lo_cpl, lo_csm, hi_cpl, hi_csm);
            Err(InternalHeaderParseError::NotFound)
        }
    }

    pub fn rom_size_in_kb(&self) -> u32 {
        let exponent = self.rom_size as u32;
        2u32.pow(exponent)
    }

    pub fn sram_size_in_kb(&self) -> u32 {
        match self.sram_size as u32 {
            0 => 0,
            exponent => 2u32.pow(exponent),
        }
    }
}

impl fmt::Display for MapMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use MapMode::*;
        write!(f, "{}", match self {
            SlowLoRom => "LoROM",
            SlowHiRom => "HiROM",
            SlowExLoRom => "ExLoROM",
            SlowSa1LoRom => "SA-1 LoROM",
            SlowExHiRom => "ExHiROM",
            SlowSa1HiRom => "SA-1 HiROM",
            FastLoRom => "Fast LoROM",
            FastHiRom => "Fast HiROM",
            FastExLoRom => "Fast ExLoROM",
            FastSa1LoRom => "Fast SA-1 LoROM",
            FastExHiRom => "Fast ExHiROM",
            FastSa1HiRom => "Fast SA-1 HiROM",
        })
    }
}

#[rustfmt::skip]
impl MapMode {
    pub fn as_u8(&self) -> u8 { (*self).into() }
    /// Nintendo's "Mode 2x" number: the low nibble of the map-mode byte.
    pub fn mode_num(&self) -> u8 { self.as_u8() & 0x0F }
    pub fn is_slow(&self)    -> bool { (self.as_u8() & 0b010000) == 0 }
    pub fn is_fast(&self)    -> bool { !self.is_slow() }
    /// SA-1 packs (Mode 23/25) expose their ROM to the S-CPU with plain
    /// LoROM/HiROM bus addressing, so an SA-1 mode still counts as
    /// LoROM/HiROM for address conversion.
    pub fn is_sa1(&self)     -> bool { matches!(self.mode_num(), 0x03 | 0x05) }
    pub fn is_lorom(&self)   -> bool { matches!(self.mode_num(), 0x00 | 0x03) }
    pub fn is_hirom(&self)   -> bool { matches!(self.mode_num(), 0x01 | 0x05) }
    pub fn is_exlorom(&self) -> bool { self.mode_num() == 0x02 }
    pub fn is_exhirom(&self) -> bool { self.mode_num() == 0x04 }
}

impl fmt::Display for RomType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use RomType::*;
        let self_as_byte: u8 = (*self).into();
        write!(f, "{}", match self {
            Rom => String::from("ROM"),
            RomRam => String::from("ROM + RAM"),
            RomRamSram => String::from("ROM + RAM + SRAM"),
            _ => format!(
                "ROM + {}{}",
                match self_as_byte & 0xF0 {
                    0x00 => "DSP",
                    0x10 => "SuperFX",
                    0x20 => "OBC-1",
                    0x30 => "SA-1",
                    0x40 => "SDD-1",
                    0x50 => "S-RTC",
                    0xE0 => "Other expansion chip",
                    0xF0 => "Custom expansion chip",
                    _ => "Unknown expansion chip",
                },
                match self_as_byte & 0x0F {
                    0x3 => "",
                    0x4 => " + RAM",
                    0x5 => " + RAM + SRAM",
                    0x6 => " + SRAM",
                    _ => " + Unknown memory chip",
                }
            ),
        })
    }
}

impl fmt::Display for RegionCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use RegionCode::*;
        write!(f, "{}", match self {
            Japan => "Japan",
            NorthAmerica => "North America",
            Europe => "Europe",
            Sweden => "Sweden",
            Finland => "Finland",
            Denmark => "Denmark",
            France => "France",
            Netherlands => "Netherlands",
            Spain => "Spain",
            Germany => "Germany",
            Italy => "Italy",
            China => "China",
            Indonesia => "Indonesia",
            Korea => "Korea",
            Global => "Global",
            Canada => "Canada",
            Brazil => "Brazil",
            Australia => "Australia",
            Other1 => "Other (1)",
            Other2 => "Other (2)",
            Other3 => "Other (3)",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snes_utils::rom::Rom;

    /// Build a synthetic in-memory ROM with a valid internal header at
    /// `header_base` (0x7FC0 = LoROM/ExLoROM/SA-1-LoROM spot,
    /// 0xFFC0 = HiROM/ExHiROM/SA-1-HiROM spot). The checksum/complement pair
    /// is consistent so `find()` accepts it. No real ROM data is involved.
    fn synthetic_rom(header_base: usize, name: &str, map_mode: u8, rom_type: u8) -> Rom {
        let mut buf = vec![0u8; 0x10000];
        let mut name_field = [b' '; sizes::INTERNAL_ROM_NAME];
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(name_field.len());
        name_field[..name_len].copy_from_slice(&name_bytes[..name_len]);
        buf[header_base..header_base + sizes::INTERNAL_ROM_NAME].copy_from_slice(&name_field);
        buf[header_base + 0x15] = map_mode;
        buf[header_base + 0x16] = rom_type;
        buf[header_base + 0x17] = 0x0B; // ROM size byte: 2^11 KB = 2 MB
        buf[header_base + 0x19] = 0x01; // region: North America
                                        // Consistent pair is all `find()` checks (cpl ^ csm == 0xFFFF).
        let checksum: u16 = 0x1234;
        let complement = !checksum;
        buf[header_base + 0x1C..header_base + 0x1E].copy_from_slice(&complement.to_le_bytes());
        buf[header_base + 0x1E..header_base + 0x20].copy_from_slice(&checksum.to_le_bytes());
        Rom::new(buf).unwrap()
    }

    #[test]
    fn parses_sa1_lorom_header() {
        let rom = synthetic_rom(0x7FC0, "SA-1 TEST ROM", 0x23, 0x34);
        let header = RomInternalHeader::parse(&rom).unwrap();
        assert_eq!(header.internal_rom_name, "SA-1 TEST ROM        ");
        assert_eq!(header.map_mode, MapMode::SlowSa1LoRom);
        assert_eq!(header.map_mode.to_string(), "SA-1 LoROM");
        assert_eq!(header.rom_type, RomType::RomSa1Ram);
        assert!(header.map_mode.is_sa1());
        assert!(header.map_mode.is_lorom());
        assert!(!header.map_mode.is_hirom());
        assert!(header.map_mode.is_slow());
    }

    #[test]
    fn parses_sa1_hirom_header() {
        let rom = synthetic_rom(0xFFC0, "SA-1 HIROM TEST", 0x25, 0x35);
        let header = RomInternalHeader::parse(&rom).unwrap();
        assert_eq!(header.map_mode, MapMode::SlowSa1HiRom);
        assert_eq!(header.map_mode.to_string(), "SA-1 HiROM");
        assert_eq!(header.rom_type, RomType::RomSa1RamSram);
        assert!(header.map_mode.is_sa1());
        assert!(header.map_mode.is_hirom());
        assert!(!header.map_mode.is_lorom());
    }

    #[test]
    fn parses_fast_sa1_modes() {
        let lo = RomInternalHeader::parse(&synthetic_rom(0x7FC0, "FAST SA-1 LO", 0x33, 0x33)).unwrap();
        assert_eq!(lo.map_mode, MapMode::FastSa1LoRom);
        assert!(lo.map_mode.is_sa1() && lo.map_mode.is_lorom() && lo.map_mode.is_fast());
        assert_eq!(lo.map_mode.to_string(), "Fast SA-1 LoROM");

        let hi = RomInternalHeader::parse(&synthetic_rom(0xFFC0, "FAST SA-1 HI", 0x35, 0x36)).unwrap();
        assert_eq!(hi.map_mode, MapMode::FastSa1HiRom);
        assert!(hi.map_mode.is_sa1() && hi.map_mode.is_hirom() && hi.map_mode.is_fast());
        assert_eq!(hi.map_mode.to_string(), "Fast SA-1 HiROM");
    }

    #[test]
    fn parses_exlorom_and_exhirom_headers() {
        let exlo = RomInternalHeader::parse(&synthetic_rom(0x7FC0, "EXLOROM TEST", 0x22, 0x02)).unwrap();
        assert_eq!(exlo.map_mode, MapMode::SlowExLoRom);
        assert!(exlo.map_mode.is_exlorom());
        assert!(!exlo.map_mode.is_lorom() && !exlo.map_mode.is_hirom() && !exlo.map_mode.is_sa1());

        let exhi = RomInternalHeader::parse(&synthetic_rom(0xFFC0, "EXHIROM TEST", 0x24, 0x02)).unwrap();
        assert_eq!(exhi.map_mode, MapMode::SlowExHiRom);
        assert!(exhi.map_mode.is_exhirom());

        let fast_exlo = RomInternalHeader::parse(&synthetic_rom(0x7FC0, "FEXLOROM", 0x32, 0x02)).unwrap();
        assert_eq!(fast_exlo.map_mode, MapMode::FastExLoRom);
        assert!(fast_exlo.map_mode.is_exlorom() && fast_exlo.map_mode.is_fast());

        let fast_exhi = RomInternalHeader::parse(&synthetic_rom(0xFFC0, "FEXHIROM", 0x34, 0x02)).unwrap();
        assert_eq!(fast_exhi.map_mode, MapMode::FastExHiRom);
        assert!(fast_exhi.map_mode.is_exhirom() && fast_exhi.map_mode.is_fast());
    }

    #[test]
    fn plain_modes_still_parse() {
        let lo = RomInternalHeader::parse(&synthetic_rom(0x7FC0, "PLAIN LOROM", 0x20, 0x02)).unwrap();
        assert_eq!(lo.map_mode, MapMode::SlowLoRom);
        assert!(lo.map_mode.is_lorom() && !lo.map_mode.is_sa1());
        assert_eq!(lo.map_mode.to_string(), "LoROM");

        let hi = RomInternalHeader::parse(&synthetic_rom(0xFFC0, "PLAIN HIROM", 0x31, 0x02)).unwrap();
        assert_eq!(hi.map_mode, MapMode::FastHiRom);
        assert!(hi.map_mode.is_hirom() && hi.map_mode.is_fast());
    }

    #[test]
    fn rejects_undefined_map_mode() {
        // 0x2A is not a defined Mode 2x number; the header must not parse.
        let rom = synthetic_rom(0x7FC0, "BOGUS MODE", 0x2A, 0x02);
        assert!(matches!(RomInternalHeader::parse(&rom), Err(InternalHeaderParseError::ReadMapMode(_))));
    }

    #[test]
    fn no_valid_header_is_not_found() {
        let rom = Rom::new(vec![0u8; 0x10000]).unwrap();
        assert!(matches!(RomInternalHeader::parse(&rom), Err(InternalHeaderParseError::NotFound)));
    }
}
