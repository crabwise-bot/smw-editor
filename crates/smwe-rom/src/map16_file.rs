//! Map16 page import/export.
//!
//! Dumps and restores Map16 pages (256 tiles × 8 bytes = 0x800 bytes per
//! page) so tile definitions can be shared between ROMs or kept as backups.
//!
//! # Formats
//!
//! - Raw 0x800-byte page dumps — byte-for-byte what Lunar Magic writes as
//!   `Map16Page.bin` (each Map16 tile is 8 bytes: four little-endian 8×8
//!   tile words in upper-left, lower-left, upper-right, lower-right order).
//!   Because there is no header, a page exported here can be imported
//!   straight into Lunar Magic's Map16 page import, and vice versa.
//! - Modern `.map16` files (Lunar Magic 1.90+): the `LM16` container with a
//!   0x40-byte header, an offset+size table, tile data (8 bytes/tile), and
//!   FG act-as data (2 bytes/tile) — see `parse_modern_map16`,
//!   `serialize_modern_partial`, and `serialize_modern_full`.
//!
//! # Page model
//!
//! [`PageSel`] selects FG or BG page 0x00-0x7F. Pages 0x00/0x01 are the
//! vanilla pages (`export_page`/`import_page` keep the legacy fixed-address
//! path, including the legacy 0x10/0x11 BG numbering); pages 0x02-0x7F are
//! the Lunar Magic 1.70/2.50 expanded pages, handled by `map16_expanded`
//! (LM's own pointer-table locations when they resolve, else this editor's
//! RATS blocks). The `tileset` parameter selects which of the five Map16
//! tileset variants a vanilla foreground page belongs to (0 = Normal,
//! 1 = Castle, 2 = Rope, 3 = Underground, 4 = Switch Palace/Ghost House);
//! it is ignored for expanded pages (shared across tilesets, as in LM) and
//! for background pages.
//!
//! # Scope notes
//!
//! Vanilla SMW dispatches block behavior by hardcoded ID range (see
//! `block_behavior::category_of`); the per-FG-tile "acts like" table is
//! Lunar Magic's 1.91+ concept, stored by `map16_expanded` and carried by
//! the modern `.map16` act-as section.

use thiserror::Error;

use crate::{
    objects::{
        map16::{Block, Tile8x8},
        tilesets::TILESETS_COUNT,
    },
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::RomError,
        rom_slice::SnesSlice,
    },
    SmwRom,
};

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Tiles per Map16 page.
pub const MAP16_PAGE_TILES: usize = 256;
/// Raw bytes per Map16 page (256 tiles × 8 bytes).
pub const MAP16_PAGE_BYTES: usize = MAP16_PAGE_TILES * 8;

/// Foreground page 0: tiles 0x000-0x0FF.
pub const PAGE_FG0: u8 = 0x00;
/// Foreground page 1: tiles 0x100-0x1FF.
pub const PAGE_FG1: u8 = 0x01;
/// Background page 0: first half of the BG Map16 table (tiles 0x00-0xFF).
pub const PAGE_BG0: u8 = 0x10;
/// Background page 1: second half of the BG Map16 table (tiles 0x100-0x1FF).
pub const PAGE_BG1: u8 = 0x11;

/// SNES address of the vanilla background Map16 table (0x1000 bytes =
/// 256 tiles = two pages). Source: `Map16BGTiles` in the SMW disassembly
/// (`symbols/SMW_U.sym`: `000D9100`).
pub const BG_MAP16_TABLE_SNES: u32 = 0x0D9100;
/// Byte size of the vanilla background Map16 table.
pub const BG_MAP16_TABLE_BYTES: usize = 0x1000;

/// Human-readable name for a page number.
pub fn page_name(page: u8) -> &'static str {
    match page {
        PAGE_FG0 => "FG page 0 (tiles 000-0FF)",
        PAGE_FG1 => "FG page 1 (tiles 100-1FF)",
        PAGE_BG0 => "BG page 0 (table tiles 00-FF)",
        PAGE_BG1 => "BG page 1 (table tiles 100-1FF)",
        _ => "unknown page",
    }
}

/// True for the foreground pages (tileset-specific); false for BG pages.
pub fn page_is_foreground(page: u8) -> bool {
    matches!(page, PAGE_FG0 | PAGE_FG1)
}

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum Map16FileError {
    #[error("Unknown Map16 page {0:#04X} (want 0x00, 0x01, 0x10 or 0x11)")]
    BadPage(u8),
    #[error("Map16 page must be exactly 0x800 bytes, got {0:#X}")]
    BadPageSize(usize),
    #[error("Map16 tileset {0} out of range (0-4)")]
    BadTileset(usize),
    #[error("Buffer too small: needed {needed:#X} bytes, have {have:#X}")]
    Truncated { needed: usize, have: usize },
    #[error("Foreground tile {0:#05X} has no fixed ROM address")]
    NoTileAddress(u16),
    #[error("Invalid SNES address {0:#X}")]
    BadAddress(u32),
    #[error("ROM error: {0}")]
    Rom(#[from] RomError),
    #[error("Modern .map16 format: {0}")]
    Modern(String),
}

// -------------------------------------------------------------------------------------------------
// Page (de)serialization
// -------------------------------------------------------------------------------------------------

/// Serialize 256 blocks to the raw 0x800-byte page layout.
pub fn serialize_page(blocks: &[Block; MAP16_PAGE_TILES]) -> [u8; MAP16_PAGE_BYTES] {
    let mut out = [0u8; MAP16_PAGE_BYTES];
    for (i, b) in blocks.iter().enumerate() {
        let words = [b.upper_left.0, b.lower_left.0, b.upper_right.0, b.lower_right.0];
        for (j, w) in words.iter().enumerate() {
            out[i * 8 + j * 2..i * 8 + j * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
    }
    out
}

/// Parse 256 blocks from raw page bytes.
pub fn parse_page(data: &[u8]) -> Result<[Block; MAP16_PAGE_TILES], Map16FileError> {
    if data.len() != MAP16_PAGE_BYTES {
        return Err(Map16FileError::BadPageSize(data.len()));
    }
    let mut blocks = [Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0))); MAP16_PAGE_TILES];
    for (i, b) in blocks.iter_mut().enumerate() {
        let w = |j: usize| u16::from_le_bytes([data[i * 8 + j * 2], data[i * 8 + j * 2 + 1]]);
        *b = Block::from_tuple((Tile8x8(w(0)), Tile8x8(w(1)), Tile8x8(w(2)), Tile8x8(w(3))));
    }
    Ok(blocks)
}

// -------------------------------------------------------------------------------------------------
// Fixed-address map for vanilla foreground Map16
// -------------------------------------------------------------------------------------------------

// Tileset-specific base addresses, mirroring `objects::tilesets::data`.
const TILES_073_0FF_BASES: [u32; TILESETS_COUNT] = [0x0D8B70, 0x0DBC00, 0x0DC800, 0x0DD400, 0x0DE300];
const TILES_100_106_BASES: [u32; TILESETS_COUNT] = [0x0D8398, 0x0DC068, 0x0DCC68, 0x0DD868, 0x0DE768];
const TILES_153_16D_BASES: [u32; TILESETS_COUNT] = [0x0D9028, 0x0DC0B8, 0x0DCCB8, 0x0DD8B8, 0x0DE7B8];

/// SNES address of a foreground Map16 tile's 8 definition bytes.
///
/// This is the exact inverse of the slice layout in
/// `objects::tilesets::data` that `Tilesets::parse` reads, so export and
/// import always agree on where a tile lives.
pub fn fg_tile_snes(tile_num: u16, tileset: usize) -> Result<u32, Map16FileError> {
    if tileset >= TILESETS_COUNT {
        return Err(Map16FileError::BadTileset(tileset));
    }
    let addr = match tile_num {
        0x000..=0x072 => 0x0D8000 + tile_num as u32 * 8,
        0x073..=0x0FF => TILES_073_0FF_BASES[tileset] + (tile_num - 0x073) as u32 * 8,
        0x100..=0x106 => TILES_100_106_BASES[tileset] + (tile_num - 0x100) as u32 * 8,
        0x107..=0x110 => 0x0DC068 + (tile_num - 0x107) as u32 * 8,
        0x111..=0x152 => 0x0D83D0 + (tile_num - 0x111) as u32 * 8,
        0x153..=0x16D => TILES_153_16D_BASES[tileset] + (tile_num - 0x153) as u32 * 8,
        0x16E..=0x1C3 => 0x0D85E0 + (tile_num - 0x16E) as u32 * 8,
        0x1C4..=0x1C7 => 0x0D8890 + (tile_num - 0x1C4) as u32 * 8,
        0x1C8..=0x1EB => 0x0D88B0 + (tile_num - 0x1C8) as u32 * 8,
        0x1EC..=0x1EF => 0x0D89D0 + (tile_num - 0x1EC) as u32 * 8,
        0x1F0..=0x1FF => 0x0D89F0 + (tile_num - 0x1F0) as u32 * 8,
        _ => return Err(Map16FileError::NoTileAddress(tile_num)),
    };
    Ok(addr)
}

// -------------------------------------------------------------------------------------------------
// Export
// -------------------------------------------------------------------------------------------------

/// Export one Map16 page (raw 0x800 bytes, Lunar Magic `Map16Page.bin`
/// compatible).
///
/// `map16_tileset` (0-4) selects the tileset variant for foreground pages
/// and is ignored for background pages.
pub fn export_page(rom: &SmwRom, page: u8, map16_tileset: usize) -> Result<Vec<u8>, Map16FileError> {
    if page_is_foreground(page) {
        if map16_tileset >= TILESETS_COUNT {
            return Err(Map16FileError::BadTileset(map16_tileset));
        }
        let base_tile = if page == PAGE_FG0 { 0x000 } else { 0x100 };
        let mut blocks = [Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0))); MAP16_PAGE_TILES];
        for (i, b) in blocks.iter_mut().enumerate() {
            let tile_num = base_tile + i;
            *b = rom.map16_tilesets.get_map16_tile(tile_num, map16_tileset).unwrap_or_else(|| {
                log::warn!("No Map16 tile {tile_num:#05X} for tileset {map16_tileset}; exporting blank");
                Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0)))
            });
        }
        Ok(serialize_page(&blocks).to_vec())
    } else if matches!(page, PAGE_BG0 | PAGE_BG1) {
        let half = (page - PAGE_BG0) as u32;
        let snes = BG_MAP16_TABLE_SNES + half * MAP16_PAGE_BYTES as u32;
        let bytes = rom.rom.slice_lorom(SnesSlice::new(AddrSnes(snes), MAP16_PAGE_BYTES))?;
        Ok(bytes.to_vec())
    } else {
        Err(Map16FileError::BadPage(page))
    }
}

// -------------------------------------------------------------------------------------------------
// Import
// -------------------------------------------------------------------------------------------------

/// Convert a SNES address to a file offset in raw ROM bytes.
fn snes_to_file(addr: u32, header_offset: usize) -> Result<usize, Map16FileError> {
    let pc = AddrPc::try_from_lorom(AddrSnes(addr)).map_err(|_| Map16FileError::BadAddress(addr))?;
    Ok(pc.as_index() + header_offset)
}

/// Write `data` at SNES address `addr` in raw ROM bytes.
fn write_snes(rom_bytes: &mut [u8], addr: u32, data: &[u8], header_offset: usize) -> Result<(), Map16FileError> {
    let file = snes_to_file(addr, header_offset)?;
    let end = file
        .checked_add(data.len())
        .ok_or(Map16FileError::Truncated { needed: usize::MAX, have: rom_bytes.len() })?;
    if end > rom_bytes.len() {
        return Err(Map16FileError::Truncated { needed: end, have: rom_bytes.len() });
    }
    rom_bytes[file..end].copy_from_slice(data);
    Ok(())
}

/// Import one raw 0x800-byte page into the ROM at its fixed vanilla
/// addresses (in place — no repointing needed).
///
/// `rom_bytes` is the raw ROM image; `header_offset` is `0x200` for
/// SMC-headered ROMs, `0` otherwise.
pub fn import_page(
    rom_bytes: &mut [u8], page: u8, map16_tileset: usize, data: &[u8], header_offset: usize,
) -> Result<(), Map16FileError> {
    if data.len() != MAP16_PAGE_BYTES {
        return Err(Map16FileError::BadPageSize(data.len()));
    }
    if page_is_foreground(page) {
        if map16_tileset >= TILESETS_COUNT {
            return Err(Map16FileError::BadTileset(map16_tileset));
        }
        let base_tile = if page == PAGE_FG0 { 0x000u16 } else { 0x100u16 };
        for i in 0..MAP16_PAGE_TILES {
            let tile_num = base_tile + i as u16;
            let addr = fg_tile_snes(tile_num, map16_tileset)?;
            write_snes(rom_bytes, addr, &data[i * 8..i * 8 + 8], header_offset)?;
        }
        Ok(())
    } else if matches!(page, PAGE_BG0 | PAGE_BG1) {
        let half = (page - PAGE_BG0) as u32;
        write_snes(rom_bytes, BG_MAP16_TABLE_SNES + half * MAP16_PAGE_BYTES as u32, data, header_offset)
    } else {
        Err(Map16FileError::BadPage(page))
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snes_utils::rom::Rom;

    #[test]
    fn page_serialize_round_trip() {
        let mut blocks = [Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0))); MAP16_PAGE_TILES];
        for (i, b) in blocks.iter_mut().enumerate() {
            let w = |k: u16| Tile8x8((i as u16).wrapping_mul(0x1234).wrapping_add(k * 0x1111));
            *b = Block::from_tuple((w(0), w(1), w(2), w(3)));
        }
        let raw = serialize_page(&blocks);
        assert_eq!(raw.len(), MAP16_PAGE_BYTES);
        let back = parse_page(&raw).unwrap();
        for (a, b) in blocks.iter().zip(back.iter()) {
            assert_eq!(a.upper_left, b.upper_left);
            assert_eq!(a.lower_left, b.lower_left);
            assert_eq!(a.upper_right, b.upper_right);
            assert_eq!(a.lower_right, b.lower_right);
        }
    }

    #[test]
    fn page_rejects_wrong_size() {
        assert!(matches!(parse_page(&[0u8; 100]), Err(Map16FileError::BadPageSize(100))));
    }

    #[test]
    fn fg_tile_addresses_match_parse_slices() {
        // Spot-check against `objects::tilesets::data`.
        assert_eq!(fg_tile_snes(0x000, 0).unwrap(), 0x0D8000);
        assert_eq!(fg_tile_snes(0x072, 0).unwrap(), 0x0D8000 + 0x72 * 8);
        assert_eq!(fg_tile_snes(0x073, 0).unwrap(), 0x0D8B70);
        assert_eq!(fg_tile_snes(0x0FF, 4).unwrap(), 0x0DE300 + (0xFF - 0x73) * 8);
        assert_eq!(fg_tile_snes(0x100, 1).unwrap(), 0x0DC068);
        assert_eq!(fg_tile_snes(0x107, 0).unwrap(), 0x0DC068);
        assert_eq!(fg_tile_snes(0x111, 0).unwrap(), 0x0D83D0);
        assert_eq!(fg_tile_snes(0x153, 3).unwrap(), 0x0DD8B8);
        assert_eq!(fg_tile_snes(0x16E, 0).unwrap(), 0x0D85E0);
        assert_eq!(fg_tile_snes(0x1C4, 0).unwrap(), 0x0D8890);
        assert_eq!(fg_tile_snes(0x1C8, 0).unwrap(), 0x0D88B0);
        assert_eq!(fg_tile_snes(0x1EC, 0).unwrap(), 0x0D89D0);
        assert_eq!(fg_tile_snes(0x1F0, 0).unwrap(), 0x0D89F0);
        assert_eq!(fg_tile_snes(0x1FF, 0).unwrap(), 0x0D89F0 + 0xF * 8);
        assert!(matches!(fg_tile_snes(0x200, 0), Err(Map16FileError::NoTileAddress(_))));
        assert!(matches!(fg_tile_snes(0x000, 5), Err(Map16FileError::BadTileset(_))));
    }

    #[test]
    fn rejects_unknown_page() {
        assert!(matches!(export_page_dummy(0x42), Err(Map16FileError::BadPage(0x42))));
    }

    fn export_page_dummy(page: u8) -> Result<Vec<u8>, Map16FileError> {
        // Validates the page-number check without needing a ROM.
        if !page_is_foreground(page) && !matches!(page, PAGE_BG0 | PAGE_BG1) {
            return Err(Map16FileError::BadPage(page));
        }
        Ok(vec![])
    }

    // Real-ROM tests: need `ROM_PATH` pointing at a headerless SMW ROM.
    fn test_rom() -> Option<SmwRom> {
        let path = std::env::var("ROM_PATH").ok()?;
        SmwRom::from_file(&path).ok()
    }

    #[test]
    #[ignore]
    fn export_fg_page0_matches_rom_bytes() {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let page = export_page(&rom, PAGE_FG0, 0).unwrap();
        assert_eq!(page.len(), MAP16_PAGE_BYTES);
        // Tile 0x000 lives at PC 0x68000; the disassembly and the raw ROM
        // agree on its first bytes.
        let raw = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        assert_eq!(&page[0..8], &raw[0x68000..0x68008]);
        // And the parsed/exported tile 0x073 matches its tileset-0 address.
        assert_eq!(&page[0x73 * 8..0x73 * 8 + 8], &raw[0x68B70..0x68B78]);
    }

    #[test]
    #[ignore]
    fn export_bg_pages_match_rom_bytes() {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let raw = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        let bg0 = export_page(&rom, PAGE_BG0, 0).unwrap();
        let bg1 = export_page(&rom, PAGE_BG1, 0).unwrap();
        assert_eq!(&bg0[..], &raw[0x69100..0x69900]);
        assert_eq!(&bg1[..], &raw[0x69900..0x6A100]);
    }

    #[test]
    #[ignore]
    fn fg_page_round_trip() {
        // Export → import into a scratch copy → re-export: byte-identical.
        round_trip_page(PAGE_FG0, 0);
        round_trip_page(PAGE_FG1, 2);
    }

    #[test]
    #[ignore]
    fn bg_page_round_trip() {
        round_trip_page(PAGE_BG0, 0);
    }

    #[test]
    #[ignore]
    fn import_actually_changes_rom() {
        // Guard against a vacuous round trip: flipping one tile's bytes must
        // change the ROM at the right address and survive a re-export.
        let mut scratch = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let mut page = export_page(&rom, PAGE_FG0, 0).unwrap();
        page[0x25 * 8] ^= 0xFF;
        import_page(&mut scratch, PAGE_FG0, 0, &page, 0).unwrap();
        // Tile 0x025 is shared, at 0x0D8000 + 0x25*8 = PC 0x68128.
        let raw = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        assert_ne!(scratch[0x68128], raw[0x68128]);
        let rom2 = SmwRom::from_rom(Rom::new(scratch).unwrap()).expect("reparse modified ROM");
        let page2 = export_page(&rom2, PAGE_FG0, 0).unwrap();
        assert_eq!(page, page2);
    }

    fn round_trip_page(page: u8, tileset: usize) {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let exported = export_page(&rom, page, tileset).unwrap();

        let mut scratch = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        import_page(&mut scratch, page, tileset, &exported, 0).unwrap();

        let rom2 = SmwRom::from_rom(Rom::new(scratch).unwrap()).expect("reparse modified ROM");
        let reexported = export_page(&rom2, page, tileset).unwrap();
        assert_eq!(exported, reexported, "page {page:#04X} not byte-identical across round trip");
    }
}

// -------------------------------------------------------------------------------------------------
// Modern `.map16` file format (Lunar Magic 1.90+)
//
// Documented in Lunar Magic's own help file (`info_map16_file_format.htm`,
// shipped in the official LM 3.63 ZIP), corroborated by FuSoYa's SMWC posts
// and Underrout's open-source human-readable-map16 parser. Summary of the
// documented layout — this implementation follows it exactly:
//
// ```text
// offset  size  field
// 0x00    4     magic: "LM16"
// 0x04    2     file format version (0x100)
// 0x06    2     game id (1 = SMW)
// 0x08    2     program version (e.g. 331 = LM 3.31)
// 0x0A    2     program id (1 = LM, 0 = none/other)
// 0x0C    4     extra flags (0)
// 0x10    4     file offset of the offset+size table
// 0x14    4     size of the offset+size table (0x40)
// 0x18    4     size X: tile columns (max 0x10 for LM = one page wide)
// 0x1C    4     size Y: tile rows
// 0x20    4     base X (16x16-tile units on LM's 0x10-wide editor grid)
// 0x24    4     base Y
// 0x28    4     flags: bit0 = tileset-specific page 2 present (table idx 4),
// 0x2C    0x14  unused (0)          bit1 = full-game export, bit2 = FG-relative
//                                   (LM 2.50+), bit3 = BG-relative (LM 2.50+)
// ```
// Then an optional comment, then the offset+size table (8 entries x
// (u32 offset, u32 size)), then the data blobs:
//
// | index | partial export            | full-game export (flag bit 1)              |
// |-------|---------------------------|--------------------------------------------|
// | 0     | tile data, 8 bytes/tile   | all pages 0x00-0xFF: 0x10000 x 8 = 0x80000 |
// | 1     | act-as data, 2 bytes/tile | FG act-as: 0x8000 x 2 = 0x10000            |
// | 2     | 0                         | FG alias of index 0 (unused by parsers)    |
// | 3     | 0                         | BG alias of index 0 (unused by parsers)    |
// | 4     | 0                         | tileset-specific page 2: 15x0x100x8        |
// | 5     | 0                         | tileset-group pages 0-1: 15x2x0x100x8     |
// | 6     | 0                         | normal pipe tiles: 4x8x8 = 0x100           |
// | 7     | 0                         | diagonal pipe tiles: 8x8 = 0x40            |
//
// Page / FG-vs-BG are *derived* from base_x/base_y + the relative flags
// (there is no explicit page field): with flag bit 2 (FG-relative) the
// first tile is base_y*0x10 + base_x, i.e. FG page = base_y >> 4; with bit
// 3 (BG-relative) the first tile is 0x8000 + base_y*0x10 + base_x, i.e. BG
// page = base_y >> 4 (LM numbers BG pages 0x80-0xFF in the file; this
// editor presents them as BG pages 0x00-0x7F). Pre-2.50 files with neither
// flag use absolute coords; base_y in 0x400-0x7FF means BG-relative after
// subtracting 0x400.
//
// Old raw 0x800-byte page files (`Map16Page.bin` compatible) remain
// supported: anything that is not an `LM16` file but is exactly 0x800
// bytes long is treated as a raw single page.
// -------------------------------------------------------------------------------------------------

/// Magic bytes of the modern `.map16` container.
pub const MODERN_MAGIC: &[u8; 4] = b"LM16";
/// File format version we write (0x100).
pub const MODERN_FORMAT_VERSION: u16 = 0x100;
/// Game id for Super Mario World.
pub const MODERN_GAME_SMW: u16 = 1;
/// Program id: 0 = none/other (LM's own id is 1; the help file says other
/// programs should use 0 unless FuSoYa assigns one).
pub const MODERN_PROGRAM_OTHER: u16 = 0;
/// Header length in bytes.
pub const MODERN_HEADER_LEN: usize = 0x40;
/// Entries in the offset+size table.
pub const MODERN_TABLE_ENTRIES: usize = 8;
/// Table entry size in bytes (u32 offset + u32 size).
pub const MODERN_TABLE_ENTRY_LEN: usize = 8;

/// Header flag bit 0: tileset-specific page 2 data present (table index 4).
pub const MH_TS_PAGE2: u32 = 0x01;
/// Header flag bit 1: this is a full-game export.
pub const MH_FULL_EXPORT: u32 = 0x02;
/// Header flag bit 2 (LM 2.50+): base coords are FG-relative.
pub const MH_FG_RELATIVE: u32 = 0x04;
/// Header flag bit 3 (LM 2.50+): base coords are BG-relative.
pub const MH_BG_RELATIVE: u32 = 0x08;

/// Full-game export: tiles covered (FG pages 0x00-0x7F + BG pages 0x80-0xFF).
pub const FULL_EXPORT_TILES: usize = 0x10000;
/// Full-game export: index 0 blob size (tiles x 8 bytes).
pub const FULL_EXPORT_TILE_BYTES: usize = FULL_EXPORT_TILES * 8; // 0x80000
/// Full-game export: index 1 blob size (FG act-as x 2 bytes).
pub const FULL_EXPORT_ACT_BYTES: usize = 0x8000 * 2; // 0x10000
/// Full-game export: index 5 blob size (15 groups x 2 pages x 256 tiles x 8).
pub const FULL_EXPORT_TS_GROUP_BYTES: usize = 15 * 2 * 256 * 8; // 0xF000
/// Tileset groups in a full export (5 real, rest duplicated per the docs).
pub const FULL_EXPORT_TS_GROUPS: usize = 15;

/// Which `.map16` on-disk representation a byte buffer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Map16FileKind {
    /// Old raw 0x800-byte page dump (`Map16Page.bin` compatible).
    RawPage,
    /// Modern `LM16` container (partial or full-game export).
    Modern,
    /// Neither (wrong size and no magic).
    Unknown,
}

/// Classify a file's bytes without parsing them.
pub fn detect_map16_file_kind(data: &[u8]) -> Map16FileKind {
    if data.len() >= MODERN_MAGIC.len() && &data[0..4] == MODERN_MAGIC {
        Map16FileKind::Modern
    } else if data.len() == MAP16_PAGE_BYTES {
        Map16FileKind::RawPage
    } else {
        Map16FileKind::Unknown
    }
}

/// One entry of the offset+size table.
#[derive(Debug, Clone, Copy)]
pub struct ModernSection {
    pub offset: u32,
    pub size:   u32,
}

/// A parsed modern `.map16` file: header fields plus the data blobs we use
/// (tile data = section 0, act-as = section 1, tileset-group pages =
/// section 5).
#[derive(Debug, Clone)]
pub struct ModernMap16 {
    pub program_version: u16,
    pub program_id:      u16,
    pub size_x:          u32,
    pub size_y:          u32,
    pub base_x:          u32,
    pub base_y:          u32,
    pub flags:           u32,
    pub comment:         String,
    pub sections:        [ModernSection; MODERN_TABLE_ENTRIES],
    /// Section 0 bytes (8 bytes/tile).
    pub tile_data:       Vec<u8>,
    /// Section 1 bytes (2 bytes/tile, FG act-as), possibly empty.
    pub act_data:        Vec<u8>,
    /// Section 5 bytes (tileset-group pages 0-1), possibly empty.
    pub ts_group_data:   Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ModernMap16Error {
    #[error("Not a modern .map16 file (missing LM16 magic)")]
    BadMagic,
    #[error("Unsupported .map16 format version {0:#06X} (want 0x100)")]
    BadVersion(u16),
    #[error("Not an SMW .map16 file (game id {0})")]
    BadGame(u16),
    #[error("Truncated .map16 file: {0}")]
    Truncated(String),
    #[error("Invalid offset+size table entry {0}: offset {1:#X} size {2:#X}")]
    BadSection(usize, u32, u32),
    #[error("Full-game export has wrong tile blob size {0:#X} (want 0x80000)")]
    BadFullTileSize(usize),
}

impl From<ModernMap16Error> for Map16FileError {
    fn from(e: ModernMap16Error) -> Self {
        Map16FileError::Modern(e.to_string())
    }
}

fn read_u16_le(data: &[u8], off: usize) -> Result<u16, ModernMap16Error> {
    data.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| ModernMap16Error::Truncated(format!("u16 at {off:#X}")))
}

fn read_u32_le(data: &[u8], off: usize) -> Result<u32, ModernMap16Error> {
    data.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| ModernMap16Error::Truncated(format!("u32 at {off:#X}")))
}

/// Parse a modern `LM16` container (partial or full-game export).
pub fn parse_modern_map16(data: &[u8]) -> Result<ModernMap16, ModernMap16Error> {
    if data.len() < MODERN_HEADER_LEN || &data[0..4] != MODERN_MAGIC {
        return Err(ModernMap16Error::BadMagic);
    }
    let version = read_u16_le(data, 0x04)?;
    if version != MODERN_FORMAT_VERSION {
        return Err(ModernMap16Error::BadVersion(version));
    }
    let game = read_u16_le(data, 0x06)?;
    if game != MODERN_GAME_SMW {
        return Err(ModernMap16Error::BadGame(game));
    }
    let program_version = read_u16_le(data, 0x08)?;
    let program_id = read_u16_le(data, 0x0A)?;
    let table_offset = read_u32_le(data, 0x10)? as usize;
    let table_size = read_u32_le(data, 0x14)? as usize;
    let size_x = read_u32_le(data, 0x18)?;
    let size_y = read_u32_le(data, 0x1C)?;
    let base_x = read_u32_le(data, 0x20)?;
    let base_y = read_u32_le(data, 0x24)?;
    let flags = read_u32_le(data, 0x28)?;
    if table_size != MODERN_TABLE_ENTRIES * MODERN_TABLE_ENTRY_LEN {
        return Err(ModernMap16Error::Truncated(format!("offset+size table size {table_size:#X}")));
    }
    let table_end = table_offset
        .checked_add(table_size)
        .ok_or_else(|| ModernMap16Error::Truncated("offset+size table range overflows".to_string()))?;
    if table_end > data.len() {
        return Err(ModernMap16Error::Truncated(format!(
            "offset+size table ends at {table_end:#X}, file is {:#X} bytes",
            data.len()
        )));
    }
    // Comment = bytes between the header and the table (LM writes an ASCII
    // comment there; may be empty).
    let comment = String::from_utf8_lossy(data.get(MODERN_HEADER_LEN..table_offset).unwrap_or(&[]))
        .trim_matches('\0')
        .to_string();

    let mut sections = [ModernSection { offset: 0, size: 0 }; MODERN_TABLE_ENTRIES];
    for (i, s) in sections.iter_mut().enumerate() {
        let off = table_offset + i * MODERN_TABLE_ENTRY_LEN;
        s.offset = read_u32_le(data, off)?;
        s.size = read_u32_le(data, off + 4)?;
    }
    let section_bytes = |idx: usize| -> Result<Vec<u8>, ModernMap16Error> {
        let s = sections[idx];
        if s.size == 0 {
            return Ok(Vec::new());
        }
        let start = s.offset as usize;
        let end =
            start.checked_add(s.size as usize).ok_or_else(|| ModernMap16Error::BadSection(idx, s.offset, s.size))?;
        if end > data.len() {
            return Err(ModernMap16Error::BadSection(idx, s.offset, s.size));
        }
        Ok(data[start..end].to_vec())
    };
    Ok(ModernMap16 {
        program_version,
        program_id,
        size_x,
        size_y,
        base_x,
        base_y,
        flags,
        comment,
        sections,
        tile_data: section_bytes(0)?,
        act_data: section_bytes(1)?,
        ts_group_data: section_bytes(5)?,
    })
}

/// Where a partial (non-full-game) export belongs: FG or BG page 0x00-0x7F,
/// derived from the base coords + relative flags per the documented rules.
/// Returns `None` for full-game exports or sub-page selections whose origin
/// is not on a page boundary (those go through the rectangle importer).
pub fn modern_partial_page(m: &ModernMap16) -> Option<ModernPartialPage> {
    if m.flags & MH_FULL_EXPORT != 0 {
        return None;
    }
    let (fg, base_y) = if m.flags & MH_FG_RELATIVE != 0 {
        (true, m.base_y)
    } else if m.flags & MH_BG_RELATIVE != 0 {
        (false, m.base_y)
    } else {
        // Pre-2.50 absolute coords: base_y 0x400-0x7FF means BG-relative
        // after subtracting 0x400.
        if (0x400..0x800).contains(&m.base_y) {
            (false, m.base_y - 0x400)
        } else {
            (true, m.base_y)
        }
    };
    // Page exports sit at base_x == 0 with base_y on a page boundary.
    if m.base_x != 0 || base_y % 0x10 != 0 {
        return None;
    }
    let page = (base_y >> 4) as u8;
    if page >= 0x80 {
        return None;
    }
    Some(ModernPartialPage { fg, page })
}

/// Target of a partial-export import: an FG or BG page (0x00-0x7F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModernPartialPage {
    pub fg:   bool,
    pub page: u8,
}

/// Serialize one page (or tile rectangle) as a modern partial `.map16`
/// file: `tiles` holds 8-bytes-per-tile words in LM order (upper-left,
/// lower-left, upper-right, lower-right); `acts` holds the matching
/// per-tile act-as values (empty for BG — "background definitions have no
/// acts-like value"). `tiles.len()` must be a multiple of `size_x`.
pub fn serialize_modern_partial(
    fg: bool, page: u8, size_x: u32, tiles: &[[u8; 8]], acts: &[u16],
) -> Result<Vec<u8>, ModernMap16Error> {
    if page >= 0x80 {
        return Err(ModernMap16Error::Truncated(format!("page {page:#04X} out of range")));
    }
    if size_x == 0 || size_x > 0x10 || tiles.len() % size_x as usize != 0 {
        return Err(ModernMap16Error::Truncated(format!("bad size_x {size_x} for {} tiles", tiles.len())));
    }
    if !acts.is_empty() && acts.len() != tiles.len() {
        return Err(ModernMap16Error::Truncated(format!("acts len {} != tiles len {}", acts.len(), tiles.len())));
    }
    let size_y = tiles.len() as u32 / size_x;
    let tile_bytes: Vec<u8> = tiles.iter().flat_map(|t| t.iter().copied()).collect();
    let act_bytes: Vec<u8> = acts.iter().flat_map(|a| a.to_le_bytes()).collect();

    let mut out = vec![0u8; MODERN_HEADER_LEN];
    out[0..4].copy_from_slice(MODERN_MAGIC);
    out[0x04..0x06].copy_from_slice(&MODERN_FORMAT_VERSION.to_le_bytes());
    out[0x06..0x08].copy_from_slice(&MODERN_GAME_SMW.to_le_bytes());
    // program version/id: 0 = other program (this editor identifies itself
    // in the comment instead).
    out[0x08..0x0A].copy_from_slice(&0u16.to_le_bytes());
    out[0x0A..0x0C].copy_from_slice(&MODERN_PROGRAM_OTHER.to_le_bytes());
    let comment = b"smw-editor";
    let table_offset = (MODERN_HEADER_LEN + comment.len()) as u32;
    out[0x10..0x14].copy_from_slice(&table_offset.to_le_bytes());
    out[0x14..0x18].copy_from_slice(&(MODERN_TABLE_ENTRIES as u32 * MODERN_TABLE_ENTRY_LEN as u32).to_le_bytes());
    out[0x18..0x1C].copy_from_slice(&size_x.to_le_bytes());
    out[0x1C..0x20].copy_from_slice(&size_y.to_le_bytes());
    out[0x20..0x24].copy_from_slice(&0u32.to_le_bytes()); // base_x
    out[0x24..0x28].copy_from_slice(&((page as u32) << 4).to_le_bytes()); // base_y
    let flags = if fg { MH_FG_RELATIVE } else { MH_BG_RELATIVE };
    out[0x28..0x2C].copy_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(comment);
    // Offset+size table: section 0 = tiles, section 1 = act-as, rest zero.
    let blobs: [Vec<u8>; MODERN_TABLE_ENTRIES] =
        [tile_bytes, act_bytes, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut table = Vec::with_capacity(MODERN_TABLE_ENTRIES * MODERN_TABLE_ENTRY_LEN);
    let mut blob_off = table_offset + (MODERN_TABLE_ENTRIES * MODERN_TABLE_ENTRY_LEN) as u32;
    let mut blob_data = Vec::new();
    for data in &blobs {
        table.extend_from_slice(&blob_off.to_le_bytes());
        table.extend_from_slice(&(data.len() as u32).to_le_bytes());
        blob_off += data.len() as u32;
        blob_data.extend_from_slice(data);
    }
    out.extend_from_slice(&table);
    out.extend_from_slice(&blob_data);
    Ok(out)
}

/// Data needed to build a full-game "export ALL" file.
pub struct FullExportInput<'a> {
    /// FG pages 0x02-0x7F present in the ROM: (page, 0x800 raw bytes).
    pub fg_pages:       &'a [(u8, Vec<u8>)],
    /// BG pages 0x00-0x7F present in the ROM: (page, 0x800 raw bytes).
    /// BG pages 0x00/0x01 are the vanilla `$0D9100` table halves.
    pub bg_pages:       &'a [(u8, Vec<u8>)],
    /// Sparse FG act-as table (absent tile = identity).
    pub acts:           &'a std::collections::HashMap<u16, u16>,
    /// FG pages 0-1 per tileset group (5 groups x 2 pages x 0x800 bytes).
    pub ts_group_pages: &'a [[u8; 0x1000]; TILESETS_COUNT],
}

/// Serialize a full-game "export ALL" `.map16` file: every FG page 0x00-0x7F
/// and every BG page 0x00-0x7F (as file pages 0x80-0xFF), the FG act-as
/// table, and the tileset-group-specific FG pages 0-1.
///
/// Follows the documented container: index 0 = 0x80000 tile bytes (FG then
/// BG; the first 0x1000 bytes — FG tiles 0x000-0x1FF — are zeroed because
/// those come from the tileset-group section, matching LM's own exports),
/// index 1 = 0x10000 act-as bytes (first 0x1000 zeroed, matching LM's
/// exports), index 5 = 15 groups x 2 pages (our 5 real groups, the rest
/// duplicating group 0 per the docs). Sections 2-4, 6, 7 are zeroed —
/// parsers (including ours) do not use the aliases, and the tileset-specific
/// page 2 / pipe-tile sections are optional.
pub fn serialize_modern_full(input: &FullExportInput) -> Vec<u8> {
    // Index 0: FG pages 0x00-0x7F (0x40000) then BG pages 0x00-0x7F (0x40000).
    let mut tile_blob = vec![0u8; FULL_EXPORT_TILE_BYTES];
    // FG tiles 0x000-0x1FF (first 0x1000 bytes) stay zeroed: they come from
    // the tileset-group section (index 5), like LM's own exports.
    for (page, data) in input.fg_pages {
        if *page >= 0x02 && *page < 0x80 && data.len() == MAP16_PAGE_BYTES {
            let off = *page as usize * MAP16_PAGE_BYTES;
            tile_blob[off..off + MAP16_PAGE_BYTES].copy_from_slice(data);
        }
    }
    for (page, data) in input.bg_pages {
        if *page < 0x80 && data.len() == MAP16_PAGE_BYTES {
            let off = 0x40000 + *page as usize * MAP16_PAGE_BYTES;
            tile_blob[off..off + MAP16_PAGE_BYTES].copy_from_slice(data);
        }
    }
    // Index 1: FG act-as, 2 bytes/tile LE; absent = identity (own tile id);
    // first 0x1000 bytes zeroed like LM's exports.
    let mut act_blob = vec![0u8; FULL_EXPORT_ACT_BYTES];
    for tile in 0x200..0x8000u16 {
        let act = input.acts.get(&tile).copied().unwrap_or(tile);
        let off = tile as usize * 2;
        act_blob[off..off + 2].copy_from_slice(&act.to_le_bytes());
    }
    // Index 5: 15 groups x (page0, page1); groups 5-14 duplicate group 0.
    let mut ts_blob = vec![0u8; FULL_EXPORT_TS_GROUP_BYTES];
    for g in 0..FULL_EXPORT_TS_GROUPS {
        let src = &input.ts_group_pages[g.min(TILESETS_COUNT - 1)];
        let off = g * 0x1000;
        ts_blob[off..off + 0x1000].copy_from_slice(src);
    }

    let mut out = vec![0u8; MODERN_HEADER_LEN];
    out[0..4].copy_from_slice(MODERN_MAGIC);
    out[0x04..0x06].copy_from_slice(&MODERN_FORMAT_VERSION.to_le_bytes());
    out[0x06..0x08].copy_from_slice(&MODERN_GAME_SMW.to_le_bytes());
    out[0x08..0x0A].copy_from_slice(&0u16.to_le_bytes());
    out[0x0A..0x0C].copy_from_slice(&MODERN_PROGRAM_OTHER.to_le_bytes());
    let comment = b"smw-editor full Map16 export";
    let table_offset = (MODERN_HEADER_LEN + comment.len()) as u32;
    out[0x10..0x14].copy_from_slice(&table_offset.to_le_bytes());
    out[0x14..0x18].copy_from_slice(&(MODERN_TABLE_ENTRIES as u32 * MODERN_TABLE_ENTRY_LEN as u32).to_le_bytes());
    out[0x18..0x1C].copy_from_slice(&0x10u32.to_le_bytes()); // size_x: one page wide
    out[0x1C..0x20].copy_from_slice(&0x1000u32.to_le_bytes()); // size_y: 0x100 pages
                                                               // base 0,0; flags: full-game export.
    out[0x28..0x2C].copy_from_slice(&MH_FULL_EXPORT.to_le_bytes());
    out.extend_from_slice(comment);
    let blobs: [&[u8]; MODERN_TABLE_ENTRIES] = [&tile_blob, &act_blob, &[], &[], &[], &ts_blob, &[], &[]];
    let mut table = Vec::with_capacity(MODERN_TABLE_ENTRIES * MODERN_TABLE_ENTRY_LEN);
    let mut blob_off = table_offset + (MODERN_TABLE_ENTRIES * MODERN_TABLE_ENTRY_LEN) as u32;
    for b in &blobs {
        table.extend_from_slice(&blob_off.to_le_bytes());
        table.extend_from_slice(&(b.len() as u32).to_le_bytes());
        blob_off += b.len() as u32;
    }
    out.extend_from_slice(&table);
    for b in &blobs {
        out.extend_from_slice(b);
    }
    out
}

// -------------------------------------------------------------------------------------------------
// Expanded pages through the page import/export API
// -------------------------------------------------------------------------------------------------

/// Page selector for the new (FG/BG x 0x00-0x7F) model. FG pages 0x00/0x01
/// and BG pages 0x00/0x01 are the vanilla pages; 0x02+ are the LM 1.70/2.50
/// expanded pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSel {
    pub fg:   bool,
    pub page: u8,
}

impl PageSel {
    pub fn label(self) -> String {
        format!("{} page {:02X}", if self.fg { "FG" } else { "BG" }, self.page)
    }
}

/// Export one page (0x800 raw bytes) under the new page model, including
/// expanded pages 0x02-0x7F.
///
/// `map16_tileset` (0-4) selects the tileset variant for vanilla FG pages
/// 0x00/0x01 and is ignored for expanded pages (shared across tilesets, as
/// in Lunar Magic) and for BG pages.
pub fn export_page_sel(rom: &SmwRom, sel: PageSel, map16_tileset: usize) -> Result<Vec<u8>, Map16FileError> {
    if sel.page >= 0x80 {
        return Err(Map16FileError::BadPage(sel.page));
    }
    if sel.fg && sel.page < EXPANDED_FIRST_PAGE_ALIAS {
        // Vanilla FG pages keep the legacy fixed-address path.
        return export_page(rom, sel.page, map16_tileset);
    }
    if !sel.fg && sel.page < EXPANDED_FIRST_PAGE_ALIAS {
        // Vanilla BG pages: legacy 0x10/0x11 numbering.
        return export_page(rom, PAGE_BG0 + sel.page, map16_tileset);
    }
    // Expanded pages: read through the expanded-page model (LM pointer
    // table first, then this editor's RATS block).
    let bytes = rom.rom.bytes();
    let header_offset = if bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };
    let opt = if sel.fg {
        crate::map16_expanded::read_expanded_fg_page(bytes, header_offset, sel.page)
    } else {
        crate::map16_expanded::read_expanded_bg_page(bytes, header_offset, sel.page)
    }
    .map_err(|e| Map16FileError::Modern(e.to_string()))?;
    Ok(opt.unwrap_or([0u8; MAP16_PAGE_BYTES]).to_vec())
}

/// Alias so the expanded-page threshold reads clearly next to `PageSel`.
const EXPANDED_FIRST_PAGE_ALIAS: u8 = 0x02;

/// Import one raw 0x800-byte page under the new page model.
///
/// Vanilla pages write in place at their fixed addresses (legacy path);
/// expanded pages write to Lunar Magic's own locations when its pointer
/// table resolves, otherwise to this editor's RATS block.
pub fn import_page_sel(
    rom_bytes: &mut [u8], sel: PageSel, map16_tileset: usize, data: &[u8], header_offset: usize,
) -> Result<(), Map16FileError> {
    if data.len() != MAP16_PAGE_BYTES {
        return Err(Map16FileError::BadPageSize(data.len()));
    }
    if sel.page >= 0x80 {
        return Err(Map16FileError::BadPage(sel.page));
    }
    if sel.fg && sel.page < EXPANDED_FIRST_PAGE_ALIAS {
        return import_page(rom_bytes, sel.page, map16_tileset, data, header_offset);
    }
    if !sel.fg && sel.page < EXPANDED_FIRST_PAGE_ALIAS {
        return import_page(rom_bytes, PAGE_BG0 + sel.page, map16_tileset, data, header_offset);
    }
    if sel.fg {
        crate::map16_expanded::write_expanded_fg_page(rom_bytes, header_offset, sel.page, data)
    } else {
        crate::map16_expanded::write_expanded_bg_page(rom_bytes, header_offset, sel.page, data)
    }
    .map_err(|e| Map16FileError::Modern(e.to_string()))
}

#[cfg(test)]
mod modern_format_tests {
    use std::collections::HashMap;

    use super::*;

    fn sample_tiles() -> Vec<[u8; 8]> {
        (0..256u16)
            .map(|i| {
                let mut t = [0u8; 8];
                for (k, b) in t.iter_mut().enumerate() {
                    *b = (i.wrapping_mul(7).wrapping_add(k as u16 * 31)) as u8;
                }
                t
            })
            .collect()
    }

    #[test]
    fn partial_fg_round_trip() {
        let tiles = sample_tiles();
        let acts: Vec<u16> = (0..256).map(|i| (i as u16).wrapping_mul(3)).collect();
        let file = serialize_modern_partial(true, 0x05, 0x10, &tiles, &acts).unwrap();
        assert_eq!(detect_map16_file_kind(&file), Map16FileKind::Modern);
        let parsed = parse_modern_map16(&file).unwrap();
        assert_eq!(parsed.flags & MH_FG_RELATIVE, MH_FG_RELATIVE);
        assert_eq!(parsed.size_x, 0x10);
        assert_eq!(parsed.size_y, 0x10);
        assert_eq!(parsed.base_y, 0x50);
        let loc = modern_partial_page(&parsed).unwrap();
        assert!(loc.fg && loc.page == 0x05);
        assert_eq!(parsed.tile_data.len(), 0x800);
        assert_eq!(parsed.act_data.len(), 0x200);
        // Tile bytes preserved verbatim (8 bytes/tile, LM order).
        let expect: Vec<u8> = tiles.iter().flat_map(|t| t.iter().copied()).collect();
        assert_eq!(parsed.tile_data, expect);
        let act_words: Vec<u16> = parsed.act_data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(act_words, acts);
    }

    #[test]
    fn partial_bg_has_no_acts() {
        let tiles = sample_tiles();
        let file = serialize_modern_partial(false, 0x02, 0x10, &tiles, &[]).unwrap();
        let parsed = parse_modern_map16(&file).unwrap();
        assert_eq!(parsed.flags & MH_BG_RELATIVE, MH_BG_RELATIVE);
        assert!(parsed.act_data.is_empty());
        let loc = modern_partial_page(&parsed).unwrap();
        assert!(!loc.fg && loc.page == 0x02);
    }

    #[test]
    fn raw_page_still_detected() {
        assert_eq!(detect_map16_file_kind(&vec![0u8; 0x800]), Map16FileKind::RawPage);
        assert_eq!(detect_map16_file_kind(&vec![0u8; 0x801]), Map16FileKind::Unknown);
        assert_eq!(detect_map16_file_kind(b"LM16"), Map16FileKind::Modern);
    }

    #[test]
    fn full_export_layout() {
        let tiles = sample_tiles();
        let raw_fg: Vec<u8> = tiles.iter().flat_map(|t| t.iter().copied()).collect();
        let raw_bg = raw_fg.clone();
        let mut acts = HashMap::new();
        acts.insert(0x205u16, 0x25u16);
        let mut ts: [[u8; 0x1000]; TILESETS_COUNT] = [[0u8; 0x1000]; TILESETS_COUNT];
        ts[2][0x10] = 0xAB;
        let input = FullExportInput {
            fg_pages:       &[(0x05, raw_fg.clone()), (0x06, raw_bg.clone())],
            bg_pages:       &[(0x00, raw_bg.clone())],
            acts:           &acts,
            ts_group_pages: &ts,
        };
        let file = serialize_modern_full(&input);
        let parsed = parse_modern_map16(&file).unwrap();
        assert_eq!(parsed.flags & MH_FULL_EXPORT, MH_FULL_EXPORT);
        assert!(modern_partial_page(&parsed).is_none());
        assert_eq!(parsed.sections[0].size as usize, FULL_EXPORT_TILE_BYTES);
        assert_eq!(parsed.sections[1].size as usize, FULL_EXPORT_ACT_BYTES);
        assert_eq!(parsed.sections[5].size as usize, FULL_EXPORT_TS_GROUP_BYTES);
        // FG page 0x05 landed at 0x05*0x800 in the FG half.
        let off = 0x05 * MAP16_PAGE_BYTES;
        assert_eq!(&parsed.tile_data[off..off + MAP16_PAGE_BYTES], &raw_fg[..]);
        // FG pages 0x00/0x01 are zeroed (they live in the tileset-group section).
        assert!(parsed.tile_data[..0x1000].iter().all(|&b| b == 0));
        // BG page 0x00 landed at the start of the BG half.
        let bgoff = 0x40000;
        assert_eq!(&parsed.tile_data[bgoff..bgoff + MAP16_PAGE_BYTES], &raw_bg[..]);
        // Act-as for tile 0x205 survived; identity default elsewhere.
        let act = |t: u16| u16::from_le_bytes([parsed.act_data[t as usize * 2], parsed.act_data[t as usize * 2 + 1]]);
        assert_eq!(act(0x205), 0x25);
        assert_eq!(act(0x500), 0x500);
        // Tileset-group section: group 2 kept its marker; groups 5+ duplicated group 0.
        assert_eq!(parsed.ts_group_data[2 * 0x1000 + 0x10], 0xAB);
        assert_eq!(&parsed.ts_group_data[5 * 0x1000..6 * 0x1000], &ts[0][..]);
    }
}
