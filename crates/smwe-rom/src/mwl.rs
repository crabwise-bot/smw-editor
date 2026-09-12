//! Lunar Magic `.mwl` level file import/export.
//!
//! MWL is FuSoYa's level-exchange format for Super Mario World: one file holds
//! everything Lunar Magic needs to recreate a level (secondary header, Layer 1
//! objects, Layer 2 objects or background tilemap, sprites) plus placeholder
//! sections for palettes, secondary entrances, ExAnimation and ExGFX/bypass data.
//!
//! # Format (binary MWL v3.63, clean-room description)
//!
//! * 0x40-byte header: signature `LM`, version `u16` (3.63 = `0x0363`),
//!   directory offset `u32`, directory byte-length `u32`, flags `u32`,
//!   48-byte comment block.
//! * Directory at the offset: 8 entries of `(u32 file_offset, u32 byte_length)`.
//! * Section 0 (level info): exactly 0x40 bytes — level number `u16`,
//!   4-byte secondary header, 1-byte flags, 4-byte midway fields, main
//!   entrance X/Y, Layer 2 scroll-extension byte, the rest reserved.
//! * Sections 1-3 (Layer 1, Layer 2, sprites): 8-byte prefix of `(u32
//!   descriptor, u32 source_address)` followed by the payload (the sprite
//!   prefix is all zeros). The Layer 1 payload is the 5-byte primary header
//!   plus the terminated object stream; the Layer 2 payload is the 5-byte
//!   Layer 2 header plus the object stream, *or* — when the source address
//!   is in bank `$FF` — an 0x800-byte little-endian tilemap. The sprite
//!   payload is the 1-byte sprite header plus the terminated sprite stream.
//! * Layer 2 backgrounds are the tricky part. In the ROM they are 0x360
//!   low-byte entries (LC-RLE1) plus a single shared high byte (page 0/1)
//!   chosen by the game from the pointer value (`$E8FE` boundary in bank
//!   `$0C`). In the MWL they expand to 1,024 little-endian Map16 words:
//!   legacy entries `0x000..0x1AF` map to words `0x000..0x1AF`, entries
//!   `0x1B0..0x35F` map to words `0x200..0x3AF`, unused words are zero, and
//!   the descriptor is `$08` (high byte 0) or `$18` (high byte 1).
//!
//! # Scope of this module (v1)
//!
//! Vanilla levels only: legacy Layer 2 backgrounds and object layers,
//! sprites, primary/secondary headers. Palette, secondary entrances,
//! ExAnimation and ExGFX/bypass sections are exported empty and rejected on
//! import when non-empty. RATS-tagged and LC_LZ2/LC_LZ3-compressed streams
//! are not produced (LC-RLE1 is what the vanilla ROM uses).

use thiserror::Error;

use crate::{
    compression::lc_rle1,
    freespace,
    level::headers::{SecondaryHeader, SECONDARY_HEADER_SIZE},
    level::{Layer2Data, Level, LAYER2_HEADER_SIZE, PRIMARY_HEADER_SIZE, SPRITE_HEADER_SIZE},
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::{Rom, RomError},
        rom_slice::SnesSlice,
    },
    SmwRom,
};

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Lunar Magic version stamped into exported files (3.63).
pub const MWL_VERSION: u16 = 0x0363;

/// Byte offset of the section directory in a canonical MWL file.
pub const MWL_DIRECTORY_OFFSET: u32 = 0x40;
/// Byte offset where section data begins in a canonical MWL file.
pub const MWL_DATA_OFFSET: u32 = 0x80;

/// Section indexes in the MWL directory.
pub const SECTION_LEVEL_INFO: usize = 0;
pub const SECTION_LAYER1: usize = 1;
pub const SECTION_LAYER2: usize = 2;
pub const SECTION_SPRITES: usize = 3;
pub const SECTION_PALETTE: usize = 4;
pub const SECTION_SECONDARY_ENTRANCES: usize = 5;
pub const SECTION_EXANIMATION: usize = 6;
pub const SECTION_EXGFX_BYPASS: usize = 7;
pub const SECTION_COUNT: usize = 8;

/// Exact size of the level-info section.
pub const LEVEL_INFO_SIZE: usize = 0x40;

/// Size of the section prefix `(u32 descriptor, u32 source_address)`.
pub const SECTION_PREFIX_SIZE: usize = 8;

/// Bank byte marking a Layer 2 source address as a background tilemap.
pub const LAYER2_BG_BANK: u8 = 0xFF;
/// Pointer value (low 16 bits) at/above which the game fills the background
/// high-byte plane with page 1 instead of page 0.
pub const LAYER2_BG_HIGH_BOUNDARY: u16 = 0xE8FE;

/// Decompressed size of a legacy Layer 2 background's low-byte plane.
pub const BG_LEGACY_LEN: usize = 0x360;
/// Word count of the MWL Layer 2 background tilemap.
pub const BG_WORD_COUNT: usize = 0x400;
/// Byte size of the MWL Layer 2 background tilemap payload.
pub const BG_MWL_PAYLOAD_LEN: usize = BG_WORD_COUNT * 2;

/// Descriptor for a legacy Layer 2 background with high byte 0.
pub const BG_DESCRIPTOR_HIGH0: u32 = 0x08;
/// Descriptor for a legacy Layer 2 background with high byte 1.
pub const BG_DESCRIPTOR_HIGH1: u32 = 0x18;

/// Layer 2 source-address sentinel meaning "no Layer 2".
const LAYER2_NONE_SENTINEL: u32 = 0xFFFFFF;

/// Comment block Lunar Magic 3.63 writes (3 lines of 16 bytes).
fn mwl_attribution() -> [u8; 48] {
    let mut out = [0u8; 48];
    let line1 = b"Lunar Magic 3.63";
    let line2 = b"  @2024 FuSoYa  ";
    let line3 = b"Defender of Relm";
    out[..line1.len()].copy_from_slice(line1);
    out[16..16 + line2.len()].copy_from_slice(line2);
    out[32..32 + line3.len()].copy_from_slice(line3);
    out
}

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MwlError {
    #[error("Not an MWL file: missing 'LM' signature")]
    BadSignature,
    #[error("MWL file is truncated (needed {needed} bytes, have {have})")]
    Truncated { needed: usize, have: usize },
    #[error("MWL directory entry {0} is out of bounds")]
    BadDirectoryEntry(usize),
    #[error("Level-info section must be exactly 0x40 bytes, got {0}")]
    BadLevelInfoSize(usize),
    #[error("Section {0} prefix is truncated")]
    BadSectionPrefix(usize),
    #[error("Section {0} is shorter than its payload prefix claims")]
    BadSectionPayload(usize),
    #[error("Layer 2 background tilemap must be exactly 0x800 bytes, got {0:#X}")]
    BadBackgroundSize(usize),
    #[error("Layer 2 background words use mixed high bytes; vanilla levels use a single page")]
    MixedBackgroundHighByte,
    #[error("Section {0} is not supported by this importer yet")]
    UnsupportedSection(usize),
    #[error("Layer {0} payload is too short for its header")]
    ShortLayerPayload(u8),
    #[error("Level number {0:#X} is out of range")]
    BadLevelNumber(u32),
    #[error("No free space for {0} ({1} bytes needed)")]
    NoFreeSpace(&'static str, usize),
    #[error("Invalid SNES address {0:#X}")]
    BadAddress(u32),
    #[error("ROM error: {0}")]
    Rom(#[from] RomError),
}

// -------------------------------------------------------------------------------------------------
// Container
// -------------------------------------------------------------------------------------------------

/// A decoded `.mwl` file: header fields plus the eight raw sections.
#[derive(Debug, Clone)]
pub struct MwlFile {
    pub version:     u16,
    pub flags:       u32,
    pub attribution: [u8; 48],
    /// Raw section payloads in directory order (level info, L1, L2,
    /// sprites, palette, secondary entrances, ExAnimation, ExGFX/bypass).
    pub sections:    [Vec<u8>; SECTION_COUNT],
}

impl MwlFile {
    /// Decode an MWL file from its bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, MwlError> {
        if bytes.len() < MWL_DATA_OFFSET as usize {
            return Err(MwlError::Truncated { needed: MWL_DATA_OFFSET as usize, have: bytes.len() });
        }
        if &bytes[0..2] != b"LM" {
            return Err(MwlError::BadSignature);
        }
        let version = u16::from_le_bytes([bytes[2], bytes[3]]);
        let dir_offset = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let dir_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let mut attribution = [0u8; 48];
        attribution.copy_from_slice(&bytes[16..64]);

        if dir_len < SECTION_COUNT * 8 || dir_offset + dir_len > bytes.len() {
            return Err(MwlError::Truncated { needed: dir_offset + dir_len, have: bytes.len() });
        }
        let mut sections: [Vec<u8>; SECTION_COUNT] = Default::default();
        for i in 0..SECTION_COUNT {
            let e = dir_offset + i * 8;
            let offset = u32::from_le_bytes(bytes[e..e + 4].try_into().unwrap()) as usize;
            let length = u32::from_le_bytes(bytes[e + 4..e + 8].try_into().unwrap()) as usize;
            if offset.checked_add(length).is_none_or(|end| end > bytes.len()) {
                return Err(MwlError::BadDirectoryEntry(i));
            }
            sections[i] = bytes[offset..offset + length].to_vec();
        }
        Ok(Self { version, flags, attribution, sections })
    }

    /// Encode this file to its canonical byte layout.
    pub fn encode(&self) -> Result<Vec<u8>, MwlError> {
        let mut out = Vec::with_capacity(MWL_DATA_OFFSET as usize);
        out.extend_from_slice(b"LM");
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&MWL_DIRECTORY_OFFSET.to_le_bytes());
        out.extend_from_slice(&((SECTION_COUNT * 8) as u32).to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.attribution);
        debug_assert_eq!(out.len(), MWL_DIRECTORY_OFFSET as usize);

        // Directory.
        let mut data_offset = MWL_DATA_OFFSET as usize;
        for section in &self.sections {
            out.extend_from_slice(&(data_offset as u32).to_le_bytes());
            out.extend_from_slice(&(section.len() as u32).to_le_bytes());
            data_offset += section.len();
        }
        debug_assert_eq!(out.len(), MWL_DATA_OFFSET as usize);
        // Section data.
        for section in &self.sections {
            out.extend_from_slice(section);
        }
        Ok(out)
    }
}

// -------------------------------------------------------------------------------------------------
// Section prefix helpers
// -------------------------------------------------------------------------------------------------

/// Build a Layer 1 / Layer 2 / sprite section body: 8-byte prefix of
/// `(u32 descriptor, u32 source_address)` followed by the payload.
pub fn encode_section(descriptor: u32, source_address: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(SECTION_PREFIX_SIZE + payload.len());
    out.extend_from_slice(&descriptor.to_le_bytes());
    out.extend_from_slice(&source_address.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Split a Layer 1 / Layer 2 / sprite section body into
/// `(descriptor, source_address, payload)`.
pub fn decode_section(index: usize, section: &[u8]) -> Result<(u32, u32, &[u8]), MwlError> {
    if section.len() < SECTION_PREFIX_SIZE {
        return Err(MwlError::BadSectionPrefix(index));
    }
    let descriptor = u32::from_le_bytes(section[0..4].try_into().unwrap());
    let source = u32::from_le_bytes(section[4..8].try_into().unwrap());
    Ok((descriptor, source, &section[SECTION_PREFIX_SIZE..]))
}

// -------------------------------------------------------------------------------------------------
// Layer 2 background conversion
// -------------------------------------------------------------------------------------------------

/// Expand a legacy Layer 2 background (0x360 low-byte entries plus the shared
/// high byte) into the 1,024 little-endian Map16 words an MWL file stores.
///
/// Legacy entries `0x000..0x1AF` become words `0x000..0x1AF`, entries
/// `0x1B0..0x35F` become words `0x200..0x3AF`; all other words are zero.
pub fn bg_legacy_to_words(entries: &[u8; BG_LEGACY_LEN], high_byte: u8) -> [u16; BG_WORD_COUNT] {
    let mut words = [0u16; BG_WORD_COUNT];
    let high = (high_byte as u16) << 8;
    for i in 0..0x1B0 {
        words[i] = entries[i] as u16 | high;
    }
    for i in 0..0x1B0 {
        words[0x200 + i] = entries[0x1B0 + i] as u16 | high;
    }
    words
}

/// Collapse MWL background words back to legacy entries, requiring the
/// single shared high byte vanilla levels use.
///
/// Returns the 0x360 entries and the shared high byte.
pub fn bg_words_to_legacy(words: &[u16; BG_WORD_COUNT]) -> Result<([u8; BG_LEGACY_LEN], u8), MwlError> {
    // All non-zero words must agree on the high byte; the zero padding words
    // are skipped.
    let mut high: Option<u8> = None;
    for &w in words.iter() {
        if w == 0 {
            continue;
        }
        let h = (w >> 8) as u8;
        match high {
            None => high = Some(h),
            Some(prev) if prev == h => {}
            _ => return Err(MwlError::MixedBackgroundHighByte),
        }
    }
    let high_byte = high.unwrap_or(0);
    let mut entries = [0u8; BG_LEGACY_LEN];
    for i in 0..0x1B0 {
        entries[i] = (words[i] & 0xFF) as u8;
    }
    for i in 0..0x1B0 {
        entries[0x1B0 + i] = (words[0x200 + i] & 0xFF) as u8;
    }
    Ok((entries, high_byte))
}

/// The shared high byte the game selects for a legacy Layer 2 background
/// pointer: page 1 when the pointer is at/above `$E8FE` in bank `$0C`
/// (i.e. `$FF:E8FE` before redirection), page 0 below it.
pub fn bg_high_byte_for_pointer(ptr: u32) -> u8 {
    if (ptr & 0xFFFF) as u16 >= LAYER2_BG_HIGH_BOUNDARY { 1 } else { 0 }
}

/// The Layer 2 section descriptor for a legacy background: `$08` with the
/// high byte in bits 4-6.
pub fn bg_descriptor(high_byte: u8) -> u32 {
    BG_DESCRIPTOR_HIGH0 | ((high_byte as u32 & 1) << 4)
}

// -------------------------------------------------------------------------------------------------
// Level-info section
// -------------------------------------------------------------------------------------------------

/// The 64-byte level-info section: level number `u16`, 4-byte secondary
/// header, 1-byte flags, 4-byte midway fields, main-entrance X/Y,
/// Layer 2 scroll-extension byte, the rest reserved (zero).
pub fn encode_level_info(level_num: u16, secondary: &SecondaryHeader) -> [u8; LEVEL_INFO_SIZE] {
    let mut out = [0u8; LEVEL_INFO_SIZE];
    out[0..2].copy_from_slice(&level_num.to_le_bytes());
    out[2..6].copy_from_slice(&secondary.0);
    out
}

/// Decoded level-info section.
#[derive(Debug, Clone)]
pub struct LevelInfo {
    pub level_num: u16,
    pub secondary: SecondaryHeader,
    pub raw:       [u8; LEVEL_INFO_SIZE],
}

pub fn decode_level_info(section: &[u8]) -> Result<LevelInfo, MwlError> {
    if section.len() != LEVEL_INFO_SIZE {
        return Err(MwlError::BadLevelInfoSize(section.len()));
    }
    let level_num = u16::from_le_bytes([section[0], section[1]]);
    let mut sec = [0u8; SECONDARY_HEADER_SIZE];
    sec.copy_from_slice(&section[2..6]);
    let mut raw = [0u8; LEVEL_INFO_SIZE];
    raw.copy_from_slice(section);
    Ok(LevelInfo { level_num, secondary: SecondaryHeader(sec), raw })
}

// -------------------------------------------------------------------------------------------------
// Export
// -------------------------------------------------------------------------------------------------

/// Read the 24-bit little-endian pointer at a SNES address from raw ROM bytes.
fn read_pointer24(rom: &Rom, addr: AddrSnes) -> Result<u32, MwlError> {
    let slice = SnesSlice::new(addr, 3);
    let b = rom.slice_lorom(slice)?;
    Ok(b[0] as u32 | ((b[1] as u32) << 8) | ((b[2] as u32) << 16))
}

/// Export `level_num` (0-based, as used by this crate) to an MWL file.
pub fn export_level(rom: &SmwRom, level_num: u32) -> Result<MwlFile, MwlError> {
    if level_num as usize >= rom.levels.len() {
        return Err(MwlError::BadLevelNumber(level_num));
    }
    let level: &Level = &rom.levels[level_num as usize];

    // --- Section 0: level info ---
    let section0 = encode_level_info(level_num as u16, &level.secondary_header).to_vec();

    // --- Section 1: Layer 1 ---
    let l1_ptr = read_pointer24(&rom.rom, AddrSnes(0x05E000 + level_num * 3))?;
    let mut l1_payload = Vec::with_capacity(PRIMARY_HEADER_SIZE + level.layer1.as_bytes().len());
    l1_payload.extend_from_slice(&level.primary_header.0);
    l1_payload.extend_from_slice(level.layer1.as_bytes());
    let section1 = encode_section(0, l1_ptr, &l1_payload);

    // --- Section 2: Layer 2 ---
    let l2_ptr = read_pointer24(&rom.rom, AddrSnes(0x05E600 + level_num * 3))?;
    let section2 = if l2_ptr == LAYER2_NONE_SENTINEL {
        // No Layer 2: still emit the prefix so the section is well-formed.
        encode_section(0, l2_ptr, &[])
    } else if (l2_ptr >> 16) as u8 == LAYER2_BG_BANK {
        // Background: expand the decompressed legacy entries into words.
        let Layer2Data::Background(bg) = &level.layer2 else {
            unreachable!("layer2 pointer bank $FF but parsed data is not a background");
        };
        let high = bg_high_byte_for_pointer(l2_ptr);
        let entries: &[u8; BG_LEGACY_LEN] = bg
            .tile_ids()
            .try_into()
            .map_err(|_| MwlError::BadBackgroundSize(bg.tile_ids().len()))?;
        let words = bg_legacy_to_words(entries, high);
        let mut payload = Vec::with_capacity(BG_MWL_PAYLOAD_LEN);
        for w in words {
            payload.extend_from_slice(&w.to_le_bytes());
        }
        encode_section(bg_descriptor(high), l2_ptr, &payload)
    } else {
        // Layer 2 objects: 5-byte header + object stream. The header comes
        // from the parsed model (it is user-editable in the level editor),
        // which matches the ROM bytes it was parsed from.
        let Layer2Data::Objects { header, objects: objs } = &level.layer2 else {
            unreachable!("layer2 pointer is not $FF but parsed data is a background");
        };
        let mut payload = Vec::with_capacity(LAYER2_HEADER_SIZE + objs.as_bytes().len());
        payload.extend_from_slice(header);
        payload.extend_from_slice(objs.as_bytes());
        encode_section(0, l2_ptr, &payload)
    };

    // --- Section 3: sprites ---
    // The sprite section's 8-byte prefix is all zeros; the payload is the
    // 1-byte sprite header plus the terminated sprite stream.
    let mut spr_payload = Vec::with_capacity(SPRITE_HEADER_SIZE + level.sprite_layer.as_bytes().len());
    spr_payload.push(level.sprite_header.0);
    spr_payload.extend_from_slice(level.sprite_layer.as_bytes());
    let section3 = encode_section(0, 0, &spr_payload);

    Ok(MwlFile {
        version: MWL_VERSION,
        flags: 0,
        attribution: mwl_attribution(),
        sections: [
            section0,
            section1,
            section2,
            section3,
            Vec::new(), // palette: not supported in v1
            Vec::new(), // secondary entrances: not supported in v1
            Vec::new(), // ExAnimation: not supported in v1
            Vec::new(), // ExGFX/bypass: not supported in v1
        ],
    })
}

// -------------------------------------------------------------------------------------------------
// Import
// -------------------------------------------------------------------------------------------------

/// SNES address of the four transposed secondary-header byte tables.
const SECONDARY_HEADER_TABLES: [u32; 4] = [0x05F000, 0x05F200, 0x05F400, 0x05F600];
/// SNES address of the Layer 1 pointer table.
const LAYER1_PTR_TABLE: u32 = 0x05E000;
/// SNES address of the Layer 2 pointer table.
const LAYER2_PTR_TABLE: u32 = 0x05E600;
/// SNES address of the sprite pointer table (16-bit entries, bank `$07`).
const SPRITE_PTR_TABLE: u32 = 0x05EC00;

/// Convert a SNES address to a file offset in raw ROM bytes.
fn snes_to_file(addr: u32, header_offset: usize) -> Result<usize, MwlError> {
    let pc = AddrPc::try_from_lorom(AddrSnes(addr)).map_err(|_| MwlError::BadAddress(addr))?;
    Ok(pc.0 as usize + header_offset)
}

/// Write `data` at SNES address `addr` in raw ROM bytes.
fn write_snes(rom_bytes: &mut [u8], addr: u32, data: &[u8], header_offset: usize) -> Result<(), MwlError> {
    let file = snes_to_file(addr, header_offset)?;
    let end = file.checked_add(data.len()).ok_or(MwlError::Truncated { needed: usize::MAX, have: rom_bytes.len() })?;
    if end > rom_bytes.len() {
        return Err(MwlError::Truncated { needed: end, have: rom_bytes.len() });
    }
    rom_bytes[file..end].copy_from_slice(data);
    Ok(())
}

/// Write `data` at PC address `pc` in raw ROM bytes.
fn write_pc(rom_bytes: &mut [u8], pc: usize, data: &[u8], header_offset: usize) -> Result<(), MwlError> {
    let file = pc + header_offset;
    let end = file.checked_add(data.len()).ok_or(MwlError::Truncated { needed: usize::MAX, have: rom_bytes.len() })?;
    if end > rom_bytes.len() {
        return Err(MwlError::Truncated { needed: end, have: rom_bytes.len() });
    }
    rom_bytes[file..end].copy_from_slice(data);
    Ok(())
}

/// Find free space for `data` in LoROM bank `bank` (PC `bank * 0x8000`), write
/// it there, and return the SNES address it was written to.
fn write_to_bank(
    rom_bytes: &mut [u8],
    data: &[u8],
    bank: u8,
    label: &'static str,
    header_offset: usize,
) -> Result<u32, MwlError> {
    let pc_start = (bank as usize) * 0x8000;
    let pc = freespace::find_free_space(rom_bytes, data.len(), pc_start, header_offset)
        .ok_or(MwlError::NoFreeSpace(label, data.len()))?;
    write_pc(rom_bytes, pc, data, header_offset)?;
    Ok(((bank as u32) << 16) | ((pc % 0x8000) as u32 + 0x8000))
}

/// Import an MWL file into `target_level` (0-based), repointing Layer 1,
/// Layer 2 and sprite data into free space the same way the level editor's
/// save path does.
///
/// `rom_bytes` is the raw ROM image; `header_offset` is `0x200` for
/// SMC-headered ROMs, `0` otherwise.
pub fn import_level(
    rom_bytes: &mut [u8],
    mwl: &MwlFile,
    target_level: u32,
    header_offset: usize,
) -> Result<(), MwlError> {
    if target_level >= 0x200 {
        return Err(MwlError::BadLevelNumber(target_level));
    }

    // --- Section 0: secondary header ---
    let info = decode_level_info(&mwl.sections[SECTION_LEVEL_INFO])?;
    for (i, table) in SECONDARY_HEADER_TABLES.iter().enumerate() {
        write_snes(rom_bytes, table + target_level, &[info.secondary.0[i]], header_offset)?;
    }

    // --- Section 1: Layer 1 (5-byte primary header + object stream) ---
    {
        let (_, _, payload) = decode_section(SECTION_LAYER1, &mwl.sections[SECTION_LAYER1])?;
        if payload.len() < PRIMARY_HEADER_SIZE {
            return Err(MwlError::ShortLayerPayload(1));
        }
        let snes = write_to_bank(rom_bytes, payload, 0x05, "Layer 1 data", header_offset)?;
        let ptr = [(snes & 0xFF) as u8, ((snes >> 8) & 0xFF) as u8, ((snes >> 16) & 0xFF) as u8];
        write_snes(rom_bytes, LAYER1_PTR_TABLE + target_level * 3, &ptr, header_offset)?;
    }

    // --- Section 2: Layer 2 ---
    {
        let (_, source, payload) = decode_section(SECTION_LAYER2, &mwl.sections[SECTION_LAYER2])?;
        if source == LAYER2_NONE_SENTINEL {
            // No Layer 2: clear the pointer.
            write_snes(rom_bytes, LAYER2_PTR_TABLE + target_level * 3, &[0xFF, 0xFF, 0xFF], header_offset)?;
        } else if (source >> 16) as u8 == LAYER2_BG_BANK {
            import_background(rom_bytes, payload, target_level, header_offset)?;
        } else {
            // Layer 2 objects: 5-byte header + object stream.
            if payload.len() < 5 {
                return Err(MwlError::ShortLayerPayload(2));
            }
            let snes = write_to_bank(rom_bytes, payload, 0x05, "Layer 2 object data", header_offset)?;
            let ptr = [(snes & 0xFF) as u8, ((snes >> 8) & 0xFF) as u8, ((snes >> 16) & 0xFF) as u8];
            write_snes(rom_bytes, LAYER2_PTR_TABLE + target_level * 3, &ptr, header_offset)?;
        }
    }

    // --- Section 3: sprites (1-byte header + sprite stream) ---
    {
        let (_, _, payload) = decode_section(SECTION_SPRITES, &mwl.sections[SECTION_SPRITES])?;
        if payload.len() < SPRITE_HEADER_SIZE {
            return Err(MwlError::ShortLayerPayload(3));
        }
        let snes = write_to_bank(rom_bytes, payload, 0x07, "sprite data", header_offset)?;
        let ptr = ((snes & 0xFFFF) as u16).to_le_bytes();
        write_snes(rom_bytes, SPRITE_PTR_TABLE + target_level * 2, &ptr, header_offset)?;
    }

    // --- Sections 4-7: unsupported in v1 ---
    for (i, name) in [
        (SECTION_PALETTE, "palette"),
        (SECTION_SECONDARY_ENTRANCES, "secondary entrances"),
        (SECTION_EXANIMATION, "ExAnimation"),
        (SECTION_EXGFX_BYPASS, "ExGFX/bypass"),
    ] {
        if !mwl.sections[i].is_empty() {
            let _ = name;
            return Err(MwlError::UnsupportedSection(i));
        }
    }

    Ok(())
}

/// Import a Layer 2 background tilemap: collapse the 1,024 MWL words to the
/// 0x360 legacy entries, compress them, place the result in bank `$0C` on the
/// side of the `$E8FE` boundary the high byte requires, and point the level
/// at it with bank `$FF`.
fn import_background(
    rom_bytes: &mut [u8],
    payload: &[u8],
    target_level: u32,
    header_offset: usize,
) -> Result<(), MwlError> {
    if payload.len() != BG_MWL_PAYLOAD_LEN {
        return Err(MwlError::BadBackgroundSize(payload.len()));
    }
    let mut words = [0u16; BG_WORD_COUNT];
    for (i, w) in words.iter_mut().enumerate() {
        *w = u16::from_le_bytes([payload[2 * i], payload[2 * i + 1]]);
    }
    let (entries, high_byte) = bg_words_to_legacy(&words)?;
    if high_byte > 1 {
        return Err(MwlError::MixedBackgroundHighByte);
    }
    let compressed = lc_rle1::compress(&entries);

    // Bank $0C spans PC 0x60000..0x68000; the game fills the high-byte plane
    // with page 1 at/above SNES $0CE8FE (PC 0x668FE), page 0 below it.
    const BANK0C_PC: usize = 0x0C * 0x8000;
    const BOUNDARY_PC: usize = BANK0C_PC + (LAYER2_BG_HIGH_BOUNDARY as usize - 0x8000);
    let (pc_start, pc_end) = if high_byte == 1 {
        (BOUNDARY_PC, BANK0C_PC + 0x8000)
    } else {
        (BANK0C_PC, BOUNDARY_PC)
    };
    let pc = freespace::find_free_space_in(rom_bytes, compressed.len(), pc_start, pc_end, header_offset)
        .ok_or(MwlError::NoFreeSpace("Layer 2 background", compressed.len()))?;
    write_pc(rom_bytes, pc, &compressed, header_offset)?;

    // Bank-$FF pointer: the game redirects it into bank $0C.
    let snes = 0xFF0000 | ((pc - BANK0C_PC) as u32 + 0x8000);
    let ptr = [(snes & 0xFF) as u8, ((snes >> 8) & 0xFF) as u8, 0xFF];
    write_snes(rom_bytes, LAYER2_PTR_TABLE + target_level * 3, &ptr, header_offset)?;
    Ok(())
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_round_trip() {
        let file = MwlFile {
            version: MWL_VERSION,
            flags: 0,
            attribution: mwl_attribution(),
            sections: [
                vec![1, 2, 3],
                vec![4, 5],
                vec![],
                vec![6],
                vec![],
                vec![],
                vec![],
                vec![],
            ],
        };
        let bytes = file.encode().unwrap();
        assert_eq!(&bytes[0..2], b"LM");
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), MWL_VERSION);
        let back = MwlFile::decode(&bytes).unwrap();
        assert_eq!(back.version, MWL_VERSION);
        assert_eq!(back.sections[0], vec![1, 2, 3]);
        assert_eq!(back.sections[3], vec![6]);
        assert!(back.sections[2].is_empty());
    }

    #[test]
    fn rejects_bad_signature() {
        let mut bytes = MwlFile {
            version: MWL_VERSION,
            flags: 0,
            attribution: mwl_attribution(),
            sections: Default::default(),
        }
        .encode()
        .unwrap();
        bytes[0] = b'X';
        assert!(matches!(MwlFile::decode(&bytes), Err(MwlError::BadSignature)));
    }

    #[test]
    fn section_prefix_round_trip() {
        let body = encode_section(0x18, 0xFFD900, &[1, 2, 3]);
        let (d, s, payload) = decode_section(2, &body).unwrap();
        assert_eq!(d, 0x18);
        assert_eq!(s, 0xFFD900);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn level_info_round_trip() {
        let sec = SecondaryHeader([0x1A, 0x2B, 0x3C, 0x4D]);
        let raw = encode_level_info(0x105, &sec);
        assert_eq!(raw.len(), LEVEL_INFO_SIZE);
        let info = decode_level_info(&raw).unwrap();
        assert_eq!(info.level_num, 0x105);
        assert_eq!(info.secondary.0, sec.0);
    }

    #[test]
    fn bg_descriptor_values() {
        // Lunar Magic's own values for legacy backgrounds (verified against
        // the LM 3.63 binary's descriptor selection).
        assert_eq!(bg_descriptor(0), 0x08);
        assert_eq!(bg_descriptor(1), 0x18);
    }

    #[test]
    fn bg_high_byte_boundary() {
        assert_eq!(bg_high_byte_for_pointer(0xFFD900), 0);
        assert_eq!(bg_high_byte_for_pointer(0xFFE8FD), 0);
        assert_eq!(bg_high_byte_for_pointer(0xFFE8FE), 1);
        assert_eq!(bg_high_byte_for_pointer(0xFFFFFF), 1);
    }

    #[test]
    fn bg_words_round_trip() {
        let mut entries = [0u8; BG_LEGACY_LEN];
        for (i, e) in entries.iter_mut().enumerate() {
            *e = (i * 7 % 251) as u8;
        }
        for high in [0u8, 1] {
            let words = bg_legacy_to_words(&entries, high);
            assert_eq!(words.len(), BG_WORD_COUNT);
            // Unused bands stay zero.
            assert!(words[0x1B0..0x200].iter().all(|&w| w == 0));
            assert!(words[0x3B0..0x400].iter().all(|&w| w == 0));
            // Spot-check the two remapped bands.
            assert_eq!(words[0], entries[0] as u16 | ((high as u16) << 8));
            assert_eq!(words[0x1AF], entries[0x1AF] as u16 | ((high as u16) << 8));
            assert_eq!(words[0x200], entries[0x1B0] as u16 | ((high as u16) << 8));
            assert_eq!(words[0x3AF], entries[0x35F] as u16 | ((high as u16) << 8));
            let (back, back_high) = bg_words_to_legacy(&words).unwrap();
            assert_eq!(back, entries);
            assert_eq!(back_high, high);
        }
    }

    #[test]
    fn bg_words_reject_mixed_high_byte() {
        let mut words = [0u16; BG_WORD_COUNT];
        words[0] = 0x0012;
        words[1] = 0x0134;
        assert!(matches!(bg_words_to_legacy(&words), Err(MwlError::MixedBackgroundHighByte)));
    }

    // Real-ROM tests: need `ROM_PATH` pointing at a headerless SMW ROM.
    fn test_rom() -> Option<SmwRom> {
        let path = std::env::var("ROM_PATH").ok()?;
        SmwRom::from_file(&path).ok()
    }

    #[test]
    #[ignore]
    fn export_level_105_shape() {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let mwl = export_level(&rom, 0x105).unwrap();
        assert_eq!(mwl.version, MWL_VERSION);
        let bytes = mwl.encode().unwrap();
        let back = MwlFile::decode(&bytes).unwrap();

        // Level info: level number + secondary header round-trip.
        let info = decode_level_info(&back.sections[SECTION_LEVEL_INFO]).unwrap();
        assert_eq!(info.level_num, 0x105);
        assert_eq!(info.secondary.0, rom.levels[0x105].secondary_header.0);

        // Layer 1: 5-byte primary header + terminated object stream.
        let (d1, _, p1) = decode_section(SECTION_LAYER1, &back.sections[SECTION_LAYER1]).unwrap();
        assert_eq!(d1, 0);
        assert_eq!(&p1[..5], &rom.levels[0x105].primary_header.0);
        assert_eq!(&p1[5..], rom.levels[0x105].layer1.as_bytes());

        // Layer 2: level 105 uses a background in the vanilla ROM.
        let (d2, src2, p2) = decode_section(SECTION_LAYER2, &back.sections[SECTION_LAYER2]).unwrap();
        assert_eq!((src2 >> 16) as u8, LAYER2_BG_BANK);
        assert_eq!(p2.len(), BG_MWL_PAYLOAD_LEN);
        let high = bg_high_byte_for_pointer(src2);
        assert_eq!(d2, bg_descriptor(high));

        // Sprites: 1-byte header + terminated stream, zero prefix.
        let (d3, s3, p3) = decode_section(SECTION_SPRITES, &back.sections[SECTION_SPRITES]).unwrap();
        assert_eq!((d3, s3), (0, 0));
        assert_eq!(p3[0], rom.levels[0x105].sprite_header.0);
        assert_eq!(&p3[1..], rom.levels[0x105].sprite_layer.as_bytes());

        // Unsupported sections stay empty.
        for i in 4..SECTION_COUNT {
            assert!(back.sections[i].is_empty(), "section {i} should be empty");
        }
    }

    #[test]
    #[ignore]
    fn export_import_round_trip() {
        // Level 0x0 exercises the background path: its BG compresses small
        // enough to fit the vanilla ROM's bank-$0C free space.
        round_trip_level(0x0);
    }

    #[test]
    #[ignore]
    fn export_import_round_trip_l2_objects() {
        // Level 0x9 uses Layer 2 objects instead of a background.
        round_trip_level(0x9);
    }

    fn round_trip_level(level_num: u32) {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let mwl = export_level(&rom, level_num).unwrap();

        // Import into a scratch copy of the ROM at the same level.
        let mut bytes = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        import_level(&mut bytes, &mwl, level_num, 0).unwrap();

        // Re-export from the modified ROM: every payload must be identical
        // (only the source addresses in the prefixes change, since the data
        // was repointed into free space).
        let rom2 = SmwRom::from_rom(Rom::new(bytes).unwrap()).expect("reparse modified ROM");
        let mwl2 = export_level(&rom2, level_num).unwrap();
        assert_eq!(mwl2.sections[SECTION_LEVEL_INFO], mwl.sections[SECTION_LEVEL_INFO]);
        for &i in &[SECTION_LAYER1, SECTION_LAYER2, SECTION_SPRITES] {
            let (_, _, p1) = decode_section(i, &mwl.sections[i]).unwrap();
            let (d2, _, p2) = decode_section(i, &mwl2.sections[i]).unwrap();
            assert_eq!(p1, p2, "section {i} payload changed across import");
            if i == SECTION_LAYER2 {
                let (d1, _, _) = decode_section(i, &mwl.sections[i]).unwrap();
                assert_eq!(d1, d2, "background descriptor changed across import");
            }
        }
    }
}
