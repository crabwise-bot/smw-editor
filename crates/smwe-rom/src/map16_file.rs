//! Map16 page import/export.
//!
//! Dumps and restores Map16 pages (256 tiles × 8 bytes = 0x800 bytes per
//! page) so tile definitions can be shared between ROMs or kept as backups.
//!
//! # Format
//!
//! A single-page file is a raw 0x800-byte dump — byte-for-byte what Lunar
//! Magic writes as `Map16Page.bin` (each Map16 tile is 8 bytes: four
//! little-endian 8×8 tile words in upper-left, lower-left, upper-right,
//! lower-right order). Because there is no header, a page exported here can
//! be imported straight into Lunar Magic's Map16 page import, and vice
//! versa.
//!
//! A multi-page set file (`.s16set`, our own container) holds several pages:
//!
//! * bytes 0x00-0x05: signature `S16SET`
//! * bytes 0x06-0x07: version `u16` (currently 1)
//! * byte  0x08:      page count `u8`
//! * bytes 0x09..:    one `(page, tileset)` byte pair per page
//! * then:            `page_count` × 0x800 raw page bytes
//!
//! Page numbers: `0x00`/`0x01` are the foreground pages (tiles 0x000-0x0FF /
//! 0x100-0x1FF), `0x10`/`0x11` are the two halves of the vanilla background
//! Map16 table at SNES `$0D9100`. The `tileset` byte selects which of the
//! five Map16 tileset variants a foreground page belongs to (0 = Normal,
//! 1 = Castle, 2 = Rope, 3 = Underground, 4 = Switch Palace/Ghost House);
//! it is ignored for background pages.
//!
//! # Scope of this module (v1)
//!
//! Vanilla layouts only: the foreground Map16 lives at fixed ROM addresses
//! (see `objects::tilesets::data`), so import writes in place and needs no
//! free-space repointing. LM-expanded pages 0x02+ are not covered, nor are
//! ExGFX tile remapping or the per-block "acts like" bytes (vanilla SMW
//! dispatches block behavior by hardcoded ID range — see
//! `block_behavior::category_of`).

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

/// Signature of the multi-page `.s16set` container.
pub const SET_SIGNATURE: &[u8; 6] = b"S16SET";
/// Container format version written by this exporter.
pub const SET_VERSION: u16 = 1;

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
    #[error("Not a Map16 set file: missing 'S16SET' signature")]
    BadSignature,
    #[error("Unsupported Map16 set version {0}")]
    BadVersion(u16),
    #[error("Map16 set file is truncated (needed {needed} bytes, have {have})")]
    Truncated { needed: usize, have: usize },
    #[error("Map16 set holds no pages")]
    EmptySet,
    #[error("Foreground tile {0:#05X} has no fixed ROM address")]
    NoTileAddress(u16),
    #[error("Invalid SNES address {0:#X}")]
    BadAddress(u32),
    #[error("ROM error: {0}")]
    Rom(#[from] RomError),
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
const TILES_073_0FF_BASES: [u32; TILESETS_COUNT] =
    [0x0D8B70, 0x0DBC00, 0x0DC800, 0x0DD400, 0x0DE300];
const TILES_100_106_BASES: [u32; TILESETS_COUNT] =
    [0x0D8398, 0x0DC068, 0x0DCC68, 0x0DD868, 0x0DE768];
const TILES_153_16D_BASES: [u32; TILESETS_COUNT] =
    [0x0D9028, 0x0DC0B8, 0x0DCCB8, 0x0DD8B8, 0x0DE7B8];

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
pub fn export_page(
    rom: &SmwRom,
    page: u8,
    map16_tileset: usize,
) -> Result<Vec<u8>, Map16FileError> {
    if page_is_foreground(page) {
        if map16_tileset >= TILESETS_COUNT {
            return Err(Map16FileError::BadTileset(map16_tileset));
        }
        let base_tile = if page == PAGE_FG0 { 0x000 } else { 0x100 };
        let mut blocks = [Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0)));
            MAP16_PAGE_TILES];
        for (i, b) in blocks.iter_mut().enumerate() {
            let tile_num = base_tile + i;
            *b = rom
                .map16_tilesets
                .get_map16_tile(tile_num, map16_tileset)
                .unwrap_or_else(|| {
                    log::warn!("No Map16 tile {tile_num:#05X} for tileset {map16_tileset}; exporting blank");
                    Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0)))
                });
        }
        Ok(serialize_page(&blocks).to_vec())
    } else if matches!(page, PAGE_BG0 | PAGE_BG1) {
        let half = (page - PAGE_BG0) as u32;
        let snes = BG_MAP16_TABLE_SNES + half * MAP16_PAGE_BYTES as u32;
        let bytes = rom
            .rom
            .slice_lorom(SnesSlice::new(AddrSnes(snes), MAP16_PAGE_BYTES))?;
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
    rom_bytes: &mut [u8],
    page: u8,
    map16_tileset: usize,
    data: &[u8],
    header_offset: usize,
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
// Multi-page set container
// -------------------------------------------------------------------------------------------------

/// One page inside a `.s16set` container: page number, tileset variant
/// (foreground pages only), and the raw 0x800 page bytes.
#[derive(Debug, Clone)]
pub struct Map16SetPage {
    pub page:    u8,
    pub tileset: u8,
    pub data:    [u8; MAP16_PAGE_BYTES],
}

/// A decoded `.s16set` multi-page file.
#[derive(Debug, Clone, Default)]
pub struct Map16SetFile {
    pub pages: Vec<Map16SetPage>,
}

impl Map16SetFile {
    /// Decode a set file from its bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, Map16FileError> {
        const HEADER: usize = 9; // signature(6) + version(2) + count(1)
        if bytes.len() < HEADER || &bytes[0..6] != SET_SIGNATURE {
            return Err(Map16FileError::BadSignature);
        }
        let version = u16::from_le_bytes([bytes[6], bytes[7]]);
        if version != SET_VERSION {
            return Err(Map16FileError::BadVersion(version));
        }
        let count = bytes[8] as usize;
        if count == 0 {
            return Err(Map16FileError::EmptySet);
        }
        let table_end = HEADER + count * 2;
        let needed = table_end + count * MAP16_PAGE_BYTES;
        if bytes.len() < needed {
            return Err(Map16FileError::Truncated { needed, have: bytes.len() });
        }
        let mut pages = Vec::with_capacity(count);
        for i in 0..count {
            let page = bytes[HEADER + i * 2];
            let tileset = bytes[HEADER + i * 2 + 1];
            if !page_is_foreground(page) && !matches!(page, PAGE_BG0 | PAGE_BG1) {
                return Err(Map16FileError::BadPage(page));
            }
            if page_is_foreground(page) && tileset as usize >= TILESETS_COUNT {
                return Err(Map16FileError::BadTileset(tileset as usize));
            }
            let start = table_end + i * MAP16_PAGE_BYTES;
            let mut data = [0u8; MAP16_PAGE_BYTES];
            data.copy_from_slice(&bytes[start..start + MAP16_PAGE_BYTES]);
            pages.push(Map16SetPage { page, tileset, data });
        }
        Ok(Self { pages })
    }

    /// Encode this set file to its canonical byte layout.
    pub fn encode(&self) -> Result<Vec<u8>, Map16FileError> {
        if self.pages.is_empty() {
            return Err(Map16FileError::EmptySet);
        }
        let mut out = Vec::with_capacity(9 + self.pages.len() * (2 + MAP16_PAGE_BYTES));
        out.extend_from_slice(SET_SIGNATURE);
        out.extend_from_slice(&SET_VERSION.to_le_bytes());
        out.push(self.pages.len() as u8);
        for p in &self.pages {
            out.push(p.page);
            out.push(p.tileset);
        }
        for p in &self.pages {
            out.extend_from_slice(&p.data);
        }
        Ok(out)
    }
}

/// Export several pages into a set file.
///
/// Each entry is `(page, map16_tileset)`; the tileset is ignored for BG
/// pages.
pub fn export_set(
    rom: &SmwRom,
    pages: &[(u8, usize)],
) -> Result<Map16SetFile, Map16FileError> {
    let mut set = Map16SetFile::default();
    for &(page, tileset) in pages {
        let data_vec = export_page(rom, page, tileset)?;
        let mut data = [0u8; MAP16_PAGE_BYTES];
        data.copy_from_slice(&data_vec);
        set.pages.push(Map16SetPage { page, tileset: tileset as u8, data });
    }
    Ok(set)
}

/// Import every page of a set file into the ROM.
pub fn import_set(
    rom_bytes: &mut [u8],
    set: &Map16SetFile,
    header_offset: usize,
) -> Result<(), Map16FileError> {
    for p in &set.pages {
        import_page(rom_bytes, p.page, p.tileset as usize, &p.data, header_offset)?;
    }
    Ok(())
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
        let mut blocks = [Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0)));
            MAP16_PAGE_TILES];
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
    fn set_container_round_trip() {
        let pages = vec![
            Map16SetPage { page: PAGE_FG0, tileset: 0, data: [0x11; MAP16_PAGE_BYTES] },
            Map16SetPage { page: PAGE_FG1, tileset: 2, data: [0x22; MAP16_PAGE_BYTES] },
            Map16SetPage { page: PAGE_BG0, tileset: 0, data: [0x33; MAP16_PAGE_BYTES] },
        ];
        let set = Map16SetFile { pages };
        let bytes = set.encode().unwrap();
        assert_eq!(&bytes[0..6], b"S16SET");
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), SET_VERSION);
        let back = Map16SetFile::decode(&bytes).unwrap();
        assert_eq!(back.pages.len(), 3);
        assert_eq!(back.pages[0].page, PAGE_FG0);
        assert_eq!(back.pages[1].tileset, 2);
        assert_eq!(back.pages[2].data[0], 0x33);
    }

    #[test]
    fn set_rejects_bad_signature() {
        let mut bytes = Map16SetFile {
            pages: vec![Map16SetPage { page: PAGE_FG0, tileset: 0, data: [0; MAP16_PAGE_BYTES] }],
        }
        .encode()
        .unwrap();
        bytes[0] = b'X';
        assert!(matches!(Map16SetFile::decode(&bytes), Err(Map16FileError::BadSignature)));
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
    fn set_export_import_round_trip() {
        let rom = test_rom().expect("ROM_PATH must point at a headerless SMW ROM");
        let set = export_set(&rom, &[(PAGE_FG0, 0), (PAGE_FG1, 1), (PAGE_BG0, 0)]).unwrap();
        let bytes = set.encode().unwrap();
        let back = Map16SetFile::decode(&bytes).unwrap();

        let mut scratch = std::fs::read(std::env::var("ROM_PATH").unwrap()).unwrap();
        import_set(&mut scratch, &back, 0).unwrap();

        let rom2 = SmwRom::from_rom(Rom::new(scratch).unwrap()).expect("reparse modified ROM");
        let set2 = export_set(&rom2, &[(PAGE_FG0, 0), (PAGE_FG1, 1), (PAGE_BG0, 0)]).unwrap();
        assert_eq!(set2.pages.len(), set.pages.len());
        for (a, b) in set.pages.iter().zip(set2.pages.iter()) {
            assert_eq!(a.page, b.page);
            assert_eq!(a.data, b.data, "page {:#04X} changed across import", a.page);
        }
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
