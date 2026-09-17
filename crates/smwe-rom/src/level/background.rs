use crate::{
    compression::{lc_rle1, DecompressionError},
    freespace::find_free_space_in,
    snes_utils::addr::{AddrPc, AddrSnes, AddressError},
};

// -------------------------------------------------------------------------------------------------
// Layout
//
// A legacy SMW background is LC-RLE1-compressed data holding 0x360 Map16 block
// IDs arranged as two 16x27 halves: entries 0x000..0x1AF are the left half
// (columns 0..15) and entries 0x1B0..0x35F are the right half (columns 16..31),
// giving a 32x27 tilemap. Each entry is a Map16 block number *within the
// background's Map16 bank (page)*, 0x00..0xFF.
//
// The bank is not stored anywhere: the game fills the tilemap's high-byte
// plane with $00 or $01 based on whether the level's Layer 2 pointer (bank
// $FF) sits below or at/above SNES $0CE8FE (SMWDisX bank_05.asm, CODE_058126).
// So changing the background Map16 bank means relocating the compressed data
// across that boundary and repointing.
// -------------------------------------------------------------------------------------------------

/// Width of a legacy background tilemap, in 16x16 Map16 cells.
pub const BG_TILEMAP_WIDTH: usize = 32;
/// Height of a legacy background tilemap, in 16x16 Map16 cells.
pub const BG_TILEMAP_HEIGHT: usize = 27;
/// Legacy background payload length: 32x27 Map16 cell IDs (0x360).
pub const BG_TILEMAP_LEN: usize = BG_TILEMAP_WIDTH * BG_TILEMAP_HEIGHT;

/// SNES bank-$0C offset below which the game fills tilemap high byte $00,
/// at/above which it fills $01 (SMWDisX `bank_05.asm`, `CODE_058126`).
pub const LAYER2_BG_HIGH_BOUNDARY: u32 = 0xE8FE;

/// Returns the background Map16 bank ("page") the game would use for a Layer 2
/// pointer `l2_ptr` (bank $FF, redirected to bank $0C).
pub fn bg_high_byte_for_pointer(l2_ptr: u32) -> u8 {
    if (l2_ptr & 0xFFFF) < LAYER2_BG_HIGH_BOUNDARY {
        0
    } else {
        1
    }
}

/// Cell index for `(col, row)` in the game's two-half tilemap layout.
/// Returns `None` when the coordinates are outside 32x27.
pub fn bg_cell_index(col: u32, row: u32) -> Option<usize> {
    if col >= BG_TILEMAP_WIDTH as u32 || row >= BG_TILEMAP_HEIGHT as u32 {
        return None;
    }
    let half = (col / 16) as usize;
    let sidx = row as usize * 16 + (col % 16) as usize;
    Some(half * BG_TILEMAP_REGION + sidx)
}

/// Inverse of [`bg_cell_index`]: `(col, row)` for a cell index.
pub fn bg_cell_pos(idx: usize) -> Option<(u32, u32)> {
    if idx >= BG_TILEMAP_LEN {
        return None;
    }
    let half = idx / BG_TILEMAP_REGION;
    let sidx = idx % BG_TILEMAP_REGION;
    Some(((half as u32) * 16 + (sidx % 16) as u32, (sidx / 16) as u32))
}

/// Entries per half of the tilemap (16x27).
const BG_TILEMAP_REGION: usize = 16 * 27;

// -------------------------------------------------------------------------------------------------
// "Add Offset to Background Tiles" (Lunar Magic F9)
// -------------------------------------------------------------------------------------------------

/// Outcome of adding a signed offset to background tiles.
///
/// The offset applies in absolute block space (`page * 256 + tile`), and the
/// new Map16 bank follows the first non-empty tile in scope — this is the
/// "automatically change the BG Map16 bank when needed" behavior Lunar Magic
/// documents. A single background can only address one 256-block bank, so
/// every other tile is clamped to the chosen bank's range instead of being
/// silently reinterpreted under the other bank. (Clamping to the very bottom
/// of bank 0 yields tile 0, the erase tile.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgTileOffset {
    /// New Map16 bank (0 or 1).
    pub new_page: u8,
    /// `(cell index, new tile value)` edits to apply.
    pub edits:    Vec<(usize, u8)>,
    /// Tiles that would have crossed into the other bank and were clamped to
    /// the chosen bank's edge instead.
    pub clamped:  u32,
}

/// Pure helper for Lunar Magic's "Add Offset to Background Tiles".
///
/// `tiles` is the full `BG_TILEMAP_LEN` tilemap; `cells` are the in-scope
/// `(col, row)` cells in scan order (the selection, or the whole tilemap when
/// nothing is selected); `old_page` is the bank the tiles currently address.
/// Empty tiles (0, the erase tile) are skipped, like in Lunar Magic.
///
/// Returns `None` when `offset` is 0 or when no non-empty tile is in scope.
pub fn bg_tile_offset(tiles: &[u8], cells: &[(u32, u32)], old_page: u8, offset: i32) -> Option<BgTileOffset> {
    if offset == 0 {
        return None;
    }
    let first = cells.iter().find_map(|&(c, r)| {
        let t = *tiles.get(bg_cell_index(c, r)?)?;
        (t != 0).then_some(t)
    })?;
    let old_page = old_page.min(1);
    let abs_first = ((old_page as i32) * 256 + first as i32 + offset).clamp(0, 511);
    let new_page = (abs_first / 256) as u8;
    let (lo, hi) = (new_page as i32 * 256, new_page as i32 * 256 + 255);
    let mut edits = Vec::new();
    let mut clamped = 0u32;
    for &(c, r) in cells {
        let idx = match bg_cell_index(c, r) {
            Some(idx) => idx,
            None => continue,
        };
        let t = tiles[idx];
        if t == 0 {
            continue;
        }
        let raw = (old_page as i32) * 256 + t as i32 + offset;
        let v = raw.clamp(lo, hi) - lo;
        if v as u8 != t {
            edits.push((idx, v as u8));
        }
        if raw != raw.clamp(lo, hi) {
            clamped += 1;
        }
    }
    Some(BgTileOffset { new_page, edits, clamped })
}

// -------------------------------------------------------------------------------------------------

pub type BackgroundTileID = u8;

#[derive(Debug, Clone)]
pub struct BackgroundData {
    tile_ids:        Vec<BackgroundTileID>,
    compressed_size: usize,
    /// Background Map16 bank ("page") 0 or 1, derived from the Layer 2
    /// pointer via [`bg_high_byte_for_pointer`].
    high_byte:       u8,
}

// -------------------------------------------------------------------------------------------------

impl BackgroundData {
    /// Returns self and the number of bytes consumed by parsing.
    /// `high_byte` defaults to 0; [`crate::level::Level::parse_l2`] sets the
    /// real value from the level's Layer 2 pointer.
    pub fn read_from(input: &[u8]) -> Result<(Self, usize), DecompressionError> {
        let (tile_ids, bytes_consumed) = lc_rle1::decompress(input)?;
        Ok((Self { tile_ids, compressed_size: bytes_consumed, high_byte: 0 }, bytes_consumed))
    }

    pub fn tile_ids(&self) -> &[BackgroundTileID] {
        &self.tile_ids
    }

    pub fn compressed_size(&self) -> usize {
        self.compressed_size
    }

    /// Background Map16 bank ("page") 0 or 1.
    pub fn high_byte(&self) -> u8 {
        self.high_byte
    }

    pub fn set_high_byte(&mut self, high_byte: u8) {
        self.high_byte = high_byte.min(1);
    }
}

// -------------------------------------------------------------------------------------------------
// Saving
// -------------------------------------------------------------------------------------------------

/// Errors from [`write_background_to_rom`].
#[derive(Debug, thiserror::Error)]
pub enum BackgroundWriteError {
    #[error("level {0:03X} does not use a background layer")]
    NotABackground(u32),
    #[error("L2 pointer table out of range for level {0:03X}")]
    TableOutOfRange(u32),
    #[error("no free space in bank $0C {side} for level {level:03X} background ({needed} bytes compressed)")]
    NoFreeSpace { level: u32, side: &'static str, needed: usize },
    #[error("background data for level {0:03X} failed to decompress: {1}")]
    BadData(u32, DecompressionError),
    #[error("background tilemap must be exactly {expected} entries, got {got}")]
    BadLength { expected: usize, got: usize },
    #[error("address conversion failed: {0}")]
    Addr(#[from] AddressError),
}

/// Write `tile_ids` back to `rom_bytes` for `level_idx` (0-based), honoring
/// `want_page` (0 or 1).
///
/// Like the rest of this crate's savers this works directly on raw ROM bytes
/// (headerless PC addressing plus `header_offset` for an SMC header):
/// - compresses with LC-RLE1 and reuses the old location when the data fits
///   *and* the old location is on the correct side of the $E8FE boundary;
/// - otherwise erases the old block (fills `$FF`) and repoints the bank-$FF
///   Layer 2 pointer into free space on the correct side of the boundary —
///   below `$0CE8FE` for page 0, at/above it for page 1 — so the game fills
///   the matching tilemap high byte;
/// - fails with [`BackgroundWriteError::NoFreeSpace`] when the target side
///   has no suitable run instead of silently writing to the wrong side.
pub fn write_background_to_rom(
    rom_bytes: &mut [u8], level_idx: u32, tile_ids: &[BackgroundTileID], want_page: u8, header_offset: usize,
) -> Result<(), BackgroundWriteError> {
    if tile_ids.len() != BG_TILEMAP_LEN {
        return Err(BackgroundWriteError::BadLength { expected: BG_TILEMAP_LEN, got: tile_ids.len() });
    }
    let want_page = want_page.min(1);

    const L2_TABLE: u32 = 0x05E600;
    let tbl_pc = AddrPc::try_from_lorom(AddrSnes(L2_TABLE + level_idx * 3))?.as_index();
    let ptr_off = tbl_pc + header_offset;
    let ptr_bytes = rom_bytes.get(ptr_off..ptr_off + 3).ok_or(BackgroundWriteError::TableOutOfRange(level_idx))?;
    let l2_raw = ptr_bytes[0] as u32 | ((ptr_bytes[1] as u32) << 8) | ((ptr_bytes[2] as u32) << 16);
    if (l2_raw >> 16) as u8 != 0xFF {
        return Err(BackgroundWriteError::NotABackground(level_idx));
    }

    let old_page = bg_high_byte_for_pointer(l2_raw);
    let compressed = lc_rle1::compress(tile_ids);

    // Background lives at bank $0C with the same 16-bit offset.
    let old_snes_0c = AddrSnes((l2_raw & 0x00FFFF) | 0x0C0000);
    let old_file = AddrPc::try_from_lorom(old_snes_0c)?.as_index() + header_offset;
    let old_bytes = rom_bytes.get(old_file..).ok_or(BackgroundWriteError::TableOutOfRange(level_idx))?;
    let old_size = lc_rle1::decompress(old_bytes)
        .map(|(_, consumed)| consumed)
        .map_err(|e| BackgroundWriteError::BadData(level_idx, e))?;

    let dest = if want_page == old_page && compressed.len() <= old_size {
        old_file
    } else {
        // Relocate into free space on the correct side of the $E8FE boundary.
        let bank0c_start = AddrPc::try_from_lorom(AddrSnes(0x0C8000))?.as_index();
        let boundary_pc = bank0c_start + (LAYER2_BG_HIGH_BOUNDARY - 0x8000) as usize;
        let bank0c_end = bank0c_start + 0x8000;
        let (start, end, side) = if want_page == 0 {
            (bank0c_start, boundary_pc, "below $0CE8FE")
        } else {
            (boundary_pc, bank0c_end, "at/above $0CE8FE")
        };
        let pc = find_free_space_in(rom_bytes, compressed.len(), start, end, header_offset)
            .ok_or(BackgroundWriteError::NoFreeSpace { level: level_idx, side, needed: compressed.len() })?;
        rom_bytes[old_file..old_file + old_size].fill(0xFF);
        // Pointer stores bank $FF with the same 16-bit offset used in bank $0C.
        let new_snes_0c = AddrSnes::try_from_lorom(AddrPc(pc as u32))?;
        let new_ptr = 0xFF0000u32 | (new_snes_0c.0 & 0x00FFFF);
        rom_bytes[ptr_off..ptr_off + 3].copy_from_slice(&new_ptr.to_le_bytes()[..3]);
        pc + header_offset
    };

    rom_bytes[dest..dest + compressed.len()].copy_from_slice(&compressed);
    if dest == old_file && compressed.len() < old_size {
        rom_bytes[dest + compressed.len()..dest + old_size].fill(0xFF);
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_index_matches_game_two_half_layout() {
        // Top-left of each half.
        assert_eq!(bg_cell_index(0, 0), Some(0));
        assert_eq!(bg_cell_index(16, 0), Some(432));
        // Bottom-right of each half.
        assert_eq!(bg_cell_index(15, 26), Some(431));
        assert_eq!(bg_cell_index(31, 26), Some(863));
        // Row stride is 16 within a half.
        assert_eq!(bg_cell_index(0, 1), Some(16));
        assert_eq!(bg_cell_index(1, 0), Some(1));
        // Out of range.
        assert_eq!(bg_cell_index(32, 0), None);
        assert_eq!(bg_cell_index(0, 27), None);
        // Inverse round-trips.
        for idx in [0, 1, 15, 16, 431, 432, 863] {
            let (c, r) = bg_cell_pos(idx).unwrap();
            assert_eq!(bg_cell_index(c, r), Some(idx));
        }
        assert_eq!(bg_cell_pos(864), None);
    }

    #[test]
    fn high_byte_boundary_matches_game() {
        // Mirrors the boundary test that lived in mwl.rs.
        assert_eq!(bg_high_byte_for_pointer(0xFF_E8FD), 0);
        assert_eq!(bg_high_byte_for_pointer(0xFF_E8FE), 1);
        assert_eq!(bg_high_byte_for_pointer(0xFF_8000), 0);
        assert_eq!(bg_high_byte_for_pointer(0xFF_FFFF), 1);
    }

    #[test]
    fn tile_offset_uniform_shift_no_bank_change() {
        let mut tiles = vec![0u8; BG_TILEMAP_LEN];
        tiles[bg_cell_index(0, 0).unwrap()] = 0x10;
        tiles[bg_cell_index(1, 0).unwrap()] = 0x20;
        let cells: Vec<(u32, u32)> = (0..2).map(|c| (c, 0)).collect();
        let r = bg_tile_offset(&tiles, &cells, 0, 5).unwrap();
        assert_eq!(r.new_page, 0);
        assert_eq!(r.clamped, 0);
        assert_eq!(r.edits, vec![(bg_cell_index(0, 0).unwrap(), 0x15), (bg_cell_index(1, 0).unwrap(), 0x25)]);
    }

    #[test]
    fn tile_offset_bank_follows_first_tile_and_clamps_stragglers() {
        let mut tiles = vec![0u8; BG_TILEMAP_LEN];
        // First non-empty tile crosses into page 1; the second would stay in
        // page 0, so it must be clamped to page 1's edge rather than be
        // silently reinterpreted under page 1.
        tiles[bg_cell_index(0, 0).unwrap()] = 0xFE;
        tiles[bg_cell_index(1, 0).unwrap()] = 0x10;
        let cells: Vec<(u32, u32)> = (0..2).map(|c| (c, 0)).collect();
        let r = bg_tile_offset(&tiles, &cells, 0, 5).unwrap();
        assert_eq!(r.new_page, 1);
        assert_eq!(r.clamped, 1);
        assert_eq!(r.edits, vec![(bg_cell_index(0, 0).unwrap(), 0x03), (bg_cell_index(1, 0).unwrap(), 0x00)]);
    }

    #[test]
    fn tile_offset_clamps_at_absolute_edges() {
        let mut tiles = vec![0u8; BG_TILEMAP_LEN];
        tiles[bg_cell_index(0, 0).unwrap()] = 0x05;
        let cells = vec![(0u32, 0u32)];
        let r = bg_tile_offset(&tiles, &cells, 0, -200).unwrap();
        assert_eq!(r.new_page, 0);
        // Clamped to the bottom of bank 0: tile 0, the erase tile.
        assert_eq!(r.edits, vec![(bg_cell_index(0, 0).unwrap(), 0x00)]);
        tiles[bg_cell_index(0, 0).unwrap()] = 0xFE;
        let r = bg_tile_offset(&tiles, &cells, 1, 200).unwrap();
        assert_eq!(r.new_page, 1);
        assert_eq!(r.clamped, 1);
        assert_eq!(r.edits, vec![(bg_cell_index(0, 0).unwrap(), 0xFF)]);
    }

    #[test]
    fn tile_offset_skips_empty_and_rejects_nothing_to_do() {
        let tiles = vec![0u8; BG_TILEMAP_LEN];
        let cells = vec![(0u32, 0u32)];
        assert_eq!(bg_tile_offset(&tiles, &cells, 0, 5), None);
        let mut tiles = vec![0u8; BG_TILEMAP_LEN];
        tiles[0] = 0x10;
        assert_eq!(bg_tile_offset(&tiles, &cells, 0, 0), None);
    }

    #[test]
    fn write_rejects_wrong_length() {
        let mut rom = vec![0xFFu8; 0x80000];
        let err = write_background_to_rom(&mut rom, 0, &[0u8; 10], 0, 0).unwrap_err();
        assert!(matches!(err, BackgroundWriteError::BadLength { .. }));
    }

    #[test]
    fn write_rejects_non_background_level() {
        // Pointer table entry with bank != $FF.
        let mut rom = vec![0xFFu8; 0x40000];
        let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05E600)).unwrap().as_index();
        rom[tbl_pc] = 0x00;
        rom[tbl_pc + 1] = 0x80;
        rom[tbl_pc + 2] = 0x05;
        let err = write_background_to_rom(&mut rom, 0, &[0u8; BG_TILEMAP_LEN], 0, 0).unwrap_err();
        assert!(matches!(err, BackgroundWriteError::NotABackground(0)));
    }

    // --- Real-ROM tests: need `ROM_PATH` pointing at a headerless SMW ROM. ---

    fn test_rom_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("ROM_PATH").ok()?;
        std::fs::read(path).ok()
    }

    fn first_background_level(rom_bytes: &[u8]) -> Option<u32> {
        for level in 0..0x200u32 {
            let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05E600 + level * 3)).ok()?.as_index();
            let s = rom_bytes.get(tbl_pc..tbl_pc + 3)?;
            if s[2] == 0xFF {
                return Some(level);
            }
        }
        None
    }

    /// Edit → write → re-parse round-trips on the real ROM without changing
    /// the bank: the tile IDs and the page survive, and the data stays in
    /// place when the compressed payload still fits.
    #[test]
    #[ignore]
    fn real_rom_background_edit_round_trips() {
        let rom_bytes = test_rom_bytes().expect("ROM_PATH must point at a headerless SMW ROM");
        let level = first_background_level(&rom_bytes).expect("no background level found");

        let mut scratch = rom_bytes.clone();
        let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05E600 + level * 3)).unwrap().as_index();
        let raw = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
        let page = bg_high_byte_for_pointer(raw);
        let old_file = AddrPc::try_from_lorom(AddrSnes((raw & 0xFFFF) | 0x0C0000)).unwrap().as_index();
        let (old_bg, _) = BackgroundData::read_from(&scratch[old_file..]).unwrap();
        assert_eq!(old_bg.tile_ids().len(), BG_TILEMAP_LEN);

        // Writing the unchanged tilemap back must be a no-op repoint-wise.
        let ptr_before = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
        write_background_to_rom(&mut scratch, level, old_bg.tile_ids(), page, 0).unwrap();
        let ptr_same = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
        assert_eq!(ptr_before, ptr_same, "unchanged write must stay in place");

        // Paint a rectangle in the middle of the tilemap.
        let mut tiles = old_bg.tile_ids().to_vec();
        for row in 10..14u32 {
            for col in 8..14u32 {
                tiles[bg_cell_index(col, row).unwrap()] = 0x7B;
            }
        }
        write_background_to_rom(&mut scratch, level, &tiles, page, 0).unwrap();
        let ptr_after = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
        // Same page either way: a relocation must stay on the right side of $E8FE.
        assert_eq!(bg_high_byte_for_pointer(ptr_after), page);

        // Re-parse from the scratch ROM and compare.
        let new_file = AddrPc::try_from_lorom(AddrSnes((ptr_after & 0xFFFF) | 0x0C0000)).unwrap().as_index();
        let (reparsed, _) = BackgroundData::read_from(&scratch[new_file..]).unwrap();
        assert_eq!(reparsed.tile_ids(), tiles.as_slice());
        assert_eq!(bg_high_byte_for_pointer(ptr_after), page);
        // The painted rectangle is exactly where we put it.
        for row in 10..14u32 {
            for col in 8..14u32 {
                assert_eq!(reparsed.tile_ids()[bg_cell_index(col, row).unwrap()], 0x7B);
            }
        }
    }

    /// Relocating to the other bank must land on the correct side of the
    /// $E8FE boundary — or fail loudly when that side has no free space.
    #[test]
    #[ignore]
    fn real_rom_background_bank_change_is_bank_correct() {
        let rom_bytes = test_rom_bytes().expect("ROM_PATH must point at a headerless SMW ROM");
        let level = first_background_level(&rom_bytes).expect("no background level found");

        let mut scratch = rom_bytes.clone();
        let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05E600 + level * 3)).unwrap().as_index();
        let raw = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
        let page = bg_high_byte_for_pointer(raw);
        let other = 1 - page;

        let (old_bg, _) = {
            let f = AddrPc::try_from_lorom(AddrSnes((raw & 0xFFFF) | 0x0C0000)).unwrap().as_index();
            BackgroundData::read_from(&scratch[f..]).unwrap()
        };
        let tiles = old_bg.tile_ids().to_vec();

        match write_background_to_rom(&mut scratch, level, &tiles, other, 0) {
            Ok(()) => {
                let new_raw = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
                assert_eq!(
                    bg_high_byte_for_pointer(new_raw),
                    other,
                    "relocated pointer must select the requested bank"
                );
                let new_file = AddrPc::try_from_lorom(AddrSnes((new_raw & 0xFFFF) | 0x0C0000)).unwrap().as_index();
                let (reparsed, _) = BackgroundData::read_from(&scratch[new_file..]).unwrap();
                assert_eq!(reparsed.tile_ids(), tiles.as_slice());
            }
            Err(BackgroundWriteError::NoFreeSpace { .. }) => {
                // Honest failure: the vanilla ROM's page-1 region genuinely
                // has almost no free space. The important part is that we did
                // NOT write to the wrong side.
                let still = u32::from_le_bytes([scratch[tbl_pc], scratch[tbl_pc + 1], scratch[tbl_pc + 2], 0]);
                assert_eq!(still, raw, "failed bank change must leave the pointer untouched");
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
