//! Dynamic level dimensions — custom height for horizontal levels.
//!
//! Lunar Magic parity (LM v3.00, integrating Vitor Vilela's "Dynamic Levels"
//! ASM): the height of a horizontal level is no longer fixed at 27 tiles. It
//! is a per-level setting, chosen in the "Change Properties in Header" dialog,
//! subject to a width/height trade imposed by the game's tilemap RAM.
//!
//! # The tilemap-RAM budget
//!
//! In-game, a horizontal level's tilemap occupies one byte per tile,
//! strided 16 bytes per row per screen (16 columns). LM's own height LUT
//! (reverse-engineered; see the pipe-dream `LM_PARITY.md` reference) is
//! exactly the set of heights satisfying
//!
//! ```text
//! screens × height_tiles × 16 ≤ 0x3800        (tilemap RAM size)
//! ```
//!
//! i.e. `screens × height_tiles ≤ 896`. Spot checks against LM's LUT:
//! 32 screens → 27 tiles (`$1B0` = 27×16), 6 screens → 149 tiles
//! (`$950` = 149×16), 1 screen → 896 tiles (`$3800` = 896×16, the famous
//! one-column 896-row level). This module validates with `≤` directly, which
//! is what LM's editor does when it "refuses a pair that does not fit".
//!
//! # What this module is (and is not)
//!
//! This module models the *setting*: the per-level height, its validation,
//! its ROM storage, and the editor-side dimensions derived from it. It does
//! NOT install LM's in-game engine (the `$13D7` runtime height, the rebuilt
//! per-screen tilemap pointer tables, the object/sprite loader hooks, or the
//! redraw engine) — without that engine, a ROM plays the level at the
//! vanilla 27-tile height. The editor canvas, grid, object overlays, and
//! placement all honor the custom height so levels can be authored here and
//! played in LM (or any engine install), and the height travels with the
//! level through `.mwl` export/import.
//!
//! Tall-object placement past row 31 needs LM's extended-object 32-row band
//! mechanism (ext 01/03 band jumps); the vanilla object stream this editor
//! writes carries a 5-bit Y, so rows 32+ are renderable/authorable in the
//! canvas but not placeable into the object stream. The Level Header dialog
//! says so next to the height picker.
//!
//! # Storage format (smw-editor native, documented)
//!
//! There is no vanilla ROM structure for this (LM implements it as an ASM
//! hack plus a per-level height byte), so smw-editor stores the heights in a
//! single RATS-tagged free-space block, exactly like [`crate::exanimation`]:
//!
//! ```text
//! "SMWLVLH1"            8 bytes magic
//! version               u8 (=1)
//! level_count           u16 LE
//! per level entry:
//!   level               u16 LE (0x000-0x1FF)
//!   height              u16 LE (tiles; always != 27, see below)
//! ```
//!
//! Only non-default heights are stored (vanilla 27 is the implicit default),
//! so a ROM where every level is vanilla height carries no block at all.
//! The RATS tag is the standard `STAR` + size + ~size header LM itself uses,
//! so other tools' free-space scanners won't clobber the block.

use std::collections::BTreeMap;

use thiserror::Error;

/// Vanilla height of a horizontal level, in tiles.
pub const VANILLA_HORIZONTAL_HEIGHT_TILES: u16 = 27;

/// Size of the game's tilemap RAM in bytes (`$7E0000` region LM's engine uses).
pub const TILEMAP_RAM_BYTES: u32 = 0x3800;

/// Bytes of tilemap RAM per row per screen: 16 columns, one byte per tile.
pub const TILEMAP_ROW_STRIDE_BYTES: u32 = 16;

/// Maximum number of screens in a horizontal level (5-bit header field).
pub const MAX_SCREENS: u32 = 0x20;

/// Magic at the start of the RATS payload.
pub const LEVEL_HEIGHT_MAGIC: &[u8; 8] = b"SMWLVLH1";

/// Current payload format version.
pub const LEVEL_HEIGHT_VERSION: u8 = 1;

#[derive(Debug, Error)]
pub enum LevelHeightError {
    #[error("no level-height block in ROM")]
    NotFound,
    #[error("level-height block is corrupt: {0}")]
    Corrupt(String),
    #[error("level {0:#05X} out of range (max 0x1FF)")]
    BadLevel(u16),
    #[error("height {0} out of range 1..={1}")]
    BadHeight(u16, u16),
    #[error("level-height payload too large for a RATS block ({0} bytes)")]
    TooLarge(usize),
    #[error("no free space for level-height block ({0} bytes)")]
    NoFreeSpace(usize),
}

/// Largest height (in tiles) a horizontal level with `screens` screens may
/// use: the largest height with `screens × height × 16 ≤ 0x3800`, i.e.
/// `screens × height ≤ 896` — LM v3.00's height LUT (block B) verbatim.
/// The 32-screen entry is LM's own quirk: the budget allows 28 rows there,
/// but the LUT stores 27 (`$1B0`); this matches it instead of the division.
///
/// `screens` is clamped to `1..=32`; a zero screen count (invalid header)
/// yields the vanilla height.
pub fn max_height_tiles(screens: u32) -> u16 {
    let screens = screens.clamp(1, MAX_SCREENS);
    if screens == MAX_SCREENS {
        // LM's LUT entry for 32 screens: `$1B0` = 27 rows, not the 28 the
        // raw budget would allow.
        27
    } else {
        (TILEMAP_RAM_BYTES / (screens * TILEMAP_ROW_STRIDE_BYTES)) as u16
    }
}

/// Whether `height` tiles fits the tilemap-RAM budget for `screens` screens.
pub fn height_fits_budget(screens: u32, height: u16) -> bool {
    height >= 1 && height <= max_height_tiles(screens)
}

/// Per-level custom heights for horizontal levels.
///
/// Levels not present in the map use [`VANILLA_HORIZONTAL_HEIGHT_TILES`].
#[derive(Debug, Clone, Default)]
pub struct LevelHeights {
    heights: BTreeMap<u16, u16>,
}

impl LevelHeights {
    /// Height of `level` in tiles (vanilla 27 when unset).
    pub fn get(&self, level: u16) -> u16 {
        self.heights.get(&level).copied().unwrap_or(VANILLA_HORIZONTAL_HEIGHT_TILES)
    }

    /// Whether `level` has a non-default height stored.
    pub fn is_custom(&self, level: u16) -> bool {
        self.heights.contains_key(&level)
    }

    /// Set `level`'s height, validating it against the tilemap-RAM budget
    /// for `screens` screens. Setting the vanilla height (27) clears the
    /// entry instead of storing it.
    pub fn set(&mut self, level: u16, height: u16, screens: u32) -> Result<(), LevelHeightError> {
        if level >= 0x200 {
            return Err(LevelHeightError::BadLevel(level));
        }
        let max = max_height_tiles(screens);
        if height < 1 || height > max {
            return Err(LevelHeightError::BadHeight(height, max));
        }
        if height == VANILLA_HORIZONTAL_HEIGHT_TILES {
            self.heights.remove(&level);
        } else {
            self.heights.insert(level, height);
        }
        Ok(())
    }

    /// Remove any custom height for `level`.
    pub fn clear(&mut self, level: u16) {
        self.heights.remove(&level);
    }

    /// Number of levels with a custom height.
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    /// Whether no custom heights are stored.
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Iterate over `(level, height)` pairs in level order.
    pub fn iter(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        self.heights.iter().map(|(&l, &h)| (l, h))
    }

    /// Parse the level-height block from raw ROM bytes. Returns
    /// [`LevelHeightError::NotFound`] when no block exists yet.
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, LevelHeightError> {
        let tag = find_block(rom_bytes).ok_or(LevelHeightError::NotFound)?;
        let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
        let end = tag.saturating_add(8).saturating_add(size).saturating_add(1);
        let payload = rom_bytes
            .get(tag + 8..end)
            .ok_or_else(|| LevelHeightError::Corrupt("level-height block overruns ROM".into()))?;
        decode_payload(payload)
    }

    /// Write the data to ROM: erase any existing block (fill with `0xFF` so
    /// it reads as free space again), allocate fresh free space, and write a
    /// new RATS-tagged block. Empty data erases the block without writing a
    /// new one.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), LevelHeightError> {
        if let Some(tag) = find_block(rom_bytes) {
            let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }

        if self.heights.is_empty() {
            return Ok(());
        }

        let payload = encode_payload(self)?;
        let total = 8 + payload.len(); // RATS tag + payload
        let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
            .ok_or(LevelHeightError::NoFreeSpace(total))?;
        let file_off = pc + header_offset;
        if payload.len() > 0x10000 {
            return Err(LevelHeightError::TooLarge(payload.len()));
        }
        let size_field = (payload.len() - 1) as u16;
        rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
        rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
        rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
        rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);
        Ok(())
    }
}

fn encode_payload(data: &LevelHeights) -> Result<Vec<u8>, LevelHeightError> {
    let mut out = Vec::with_capacity(11 + data.heights.len() * 4);
    out.extend_from_slice(LEVEL_HEIGHT_MAGIC);
    out.push(LEVEL_HEIGHT_VERSION);
    out.extend_from_slice(&(data.heights.len() as u16).to_le_bytes());
    for (&level, &height) in &data.heights {
        out.extend_from_slice(&level.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
    }
    if out.len() > 0x10001 {
        return Err(LevelHeightError::TooLarge(out.len()));
    }
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<LevelHeights, LevelHeightError> {
    let corrupt = |msg: String| LevelHeightError::Corrupt(msg);
    if payload.len() < 11 {
        return Err(corrupt("payload shorter than header".into()));
    }
    if &payload[..8] != LEVEL_HEIGHT_MAGIC {
        return Err(corrupt("bad magic".into()));
    }
    if payload[8] != LEVEL_HEIGHT_VERSION {
        return Err(corrupt(format!("unsupported version {}", payload[8])));
    }
    let count = u16::from_le_bytes([payload[9], payload[10]]) as usize;
    if payload.len() != 11 + count * 4 {
        return Err(corrupt("payload length does not match level count".into()));
    }
    let mut heights = BTreeMap::new();
    for i in 0..count {
        let off = 11 + i * 4;
        let level = u16::from_le_bytes([payload[off], payload[off + 1]]);
        let height = u16::from_le_bytes([payload[off + 2], payload[off + 3]]);
        if level >= 0x200 {
            return Err(corrupt("level number out of range".into()));
        }
        if height < 1 || height > max_height_tiles(1) {
            return Err(corrupt("height out of range".into()));
        }
        if heights.insert(level, height).is_some() {
            return Err(corrupt("duplicate level entry".into()));
        }
    }
    Ok(LevelHeights { heights })
}

/// Scan `rom_bytes` for the level-height RATS block. Returns the file offset
/// of the `STAR` tag. Same shape as [`crate::exanimation`]'s scanner: the
/// claimed size must fit in the ROM and the payload must decode.
fn find_block(rom_bytes: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    while i + 16 < rom_bytes.len() {
        if &rom_bytes[i..i + 4] == b"STAR" {
            let size = u16::from_le_bytes([rom_bytes[i + 4], rom_bytes[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom_bytes[i + 6], rom_bytes[i + 7]]);
            if size as u16 ^ inv == 0xFFFF {
                let data_start = i + 8;
                let payload_end = data_start.saturating_add(size).saturating_add(1);
                if payload_end <= rom_bytes.len()
                    && data_start + 11 <= rom_bytes.len()
                    && &rom_bytes[data_start..data_start + 8] == LEVEL_HEIGHT_MAGIC
                    && decode_payload(&rom_bytes[data_start..payload_end]).is_ok()
                {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_matches_lm_lut_spot_checks() {
        // Anchors read from LM v3.00's height LUT (block B): 32 screens ->
        // 27 tiles ($1B0 = 27*16 — the budget alone would allow 28, LM stores
        // 27); 6 screens -> 149 ($950 = 149*16); 1 screen -> 896 ($3800).
        assert!(height_fits_budget(32, 27));
        assert!(!height_fits_budget(32, 28));
        assert!(height_fits_budget(6, 149));
        assert!(height_fits_budget(1, 896));
        assert_eq!(max_height_tiles(32), 27);
        assert_eq!(max_height_tiles(6), 149);
        assert_eq!(max_height_tiles(1), 896);
        // Over budget is refused.
        assert!(!height_fits_budget(32, 29));
        assert!(!height_fits_budget(6, 150));
        assert!(!height_fits_budget(1, 897));
        assert!(!height_fits_budget(4, 0));
    }

    #[test]
    fn vanilla_height_is_default_and_not_stored() {
        let mut h = LevelHeights::default();
        assert_eq!(h.get(0x105), VANILLA_HORIZONTAL_HEIGHT_TILES);
        assert!(!h.is_custom(0x105));
        h.set(0x105, 27, 4).unwrap();
        assert!(h.is_empty());
        h.set(0x105, 40, 4).unwrap();
        assert_eq!(h.get(0x105), 40);
        assert!(h.is_custom(0x105));
        h.clear(0x105);
        assert_eq!(h.get(0x105), VANILLA_HORIZONTAL_HEIGHT_TILES);
    }

    #[test]
    fn set_validates_level_and_budget() {
        let mut h = LevelHeights::default();
        assert!(matches!(h.set(0x200, 40, 4), Err(LevelHeightError::BadLevel(_))));
        assert!(matches!(h.set(0x105, 0, 4), Err(LevelHeightError::BadHeight(_, _))));
        // 4 screens -> max 224 tiles.
        assert!(matches!(h.set(0x105, 225, 4), Err(LevelHeightError::BadHeight(225, 224))));
        h.set(0x105, 224, 4).unwrap();
    }

    #[test]
    fn payload_round_trip() {
        let mut h = LevelHeights::default();
        h.set(0x105, 40, 4).unwrap();
        h.set(0x007, 896, 1).unwrap();
        let payload = encode_payload(&h).unwrap();
        let back = decode_payload(&payload).unwrap();
        assert_eq!(back.get(0x105), 40);
        assert_eq!(back.get(0x007), 896);
        assert_eq!(back.get(0x100), VANILLA_HORIZONTAL_HEIGHT_TILES);
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_payload(&[]).is_err());
        assert!(decode_payload(b"BADMAGIC\x01\x00\x00").is_err()); // bad magic
        let mut bad = encode_payload(&{
            let mut h = LevelHeights::default();
            h.set(0x105, 40, 4).unwrap();
            h
        })
        .unwrap();
        bad[8] = 0x7F; // version
        assert!(matches!(decode_payload(&bad), Err(LevelHeightError::Corrupt(_))));
        bad[8] = LEVEL_HEIGHT_VERSION;
        bad.truncate(bad.len() - 1); // truncated entry
        assert!(matches!(decode_payload(&bad), Err(LevelHeightError::Corrupt(_))));
    }

    #[test]
    fn parse_missing_block_is_not_found() {
        let rom = vec![0xFFu8; 0x8000];
        assert!(matches!(LevelHeights::parse(&rom), Err(LevelHeightError::NotFound)));
    }

    #[test]
    fn write_and_parse_round_trip_in_scratch_rom() {
        // A blank 512KB image with enough 0xFF free space.
        let mut rom = vec![0xFFu8; 0x80000];
        let mut h = LevelHeights::default();
        h.set(0x105, 40, 4).unwrap();
        h.write_to_rom(&mut rom, 0).unwrap();
        let back = LevelHeights::parse(&rom).unwrap();
        assert_eq!(back.get(0x105), 40);
        // Rewriting erases the old block: the ROM still parses exactly once.
        h.set(0x105, 48, 4).unwrap();
        h.write_to_rom(&mut rom, 0).unwrap();
        let back2 = LevelHeights::parse(&rom).unwrap();
        assert_eq!(back2.get(0x105), 48);
        assert_eq!(back2.len(), 1);
        // Empty data erases the block entirely.
        LevelHeights::default().write_to_rom(&mut rom, 0).unwrap();
        assert!(matches!(LevelHeights::parse(&rom), Err(LevelHeightError::NotFound)));
    }

    #[test]
    fn ignores_unrelated_rats_blocks() {
        let mut rom = vec![0xFFu8; 0x80000];
        // Some other tool's RATS block.
        rom[0x1000..0x1004].copy_from_slice(b"STAR");
        rom[0x1004..0x1006].copy_from_slice(&7u16.to_le_bytes());
        rom[0x1006..0x1008].copy_from_slice(&(!7u16).to_le_bytes());
        rom[0x1008..0x1010].copy_from_slice(b"OTHERMAG");
        assert!(matches!(LevelHeights::parse(&rom), Err(LevelHeightError::NotFound)));
        let mut h = LevelHeights::default();
        h.set(0x105, 40, 4).unwrap();
        h.write_to_rom(&mut rom, 0).unwrap();
        assert_eq!(LevelHeights::parse(&rom).unwrap().get(0x105), 40);
    }

    /// Real-ROM test: write a custom height into a scratch *copy* of the real
    /// ROM (the file itself is never touched), parse it back, and confirm
    /// the whole ROM still parses with the height visible on `SmwRom`.
    #[test]
    #[ignore]
    fn real_rom_write_parse_round_trip() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM for ignored tests");
        let raw = std::fs::read(path).expect("cannot read ROM");
        let header_offset = if raw.len() % 0x400 == 0x200 { 0x200 } else { 0 };
        let mut rom = raw.clone();

        // Level 0x105 (Yoshi's Island 2) is horizontal; give it a tall height
        // that fits its screen count.
        let screens = {
            let parsed = crate::SmwRom::from_rom(crate::snes_utils::rom::Rom::new(raw).unwrap()).unwrap();
            let level = &parsed.levels[0x105];
            assert!(!level.secondary_header.vertical_level());
            level.primary_header.level_length() as u32 + 1
        };
        let height = max_height_tiles(screens).min(48);
        assert!(height > VANILLA_HORIZONTAL_HEIGHT_TILES);

        let mut data = LevelHeights::default();
        data.set(0x105, height, screens).unwrap();
        data.write_to_rom(&mut rom, header_offset).unwrap();

        let back = LevelHeights::parse(&rom).unwrap();
        assert_eq!(back.get(0x105), height);
        assert_eq!(back.get(0x106), VANILLA_HORIZONTAL_HEIGHT_TILES);

        // The ROM still parses as a whole with the block present.
        let parsed = crate::SmwRom::from_rom(crate::snes_utils::rom::Rom::new(rom).unwrap()).unwrap();
        assert_eq!(parsed.level_heights.get(0x105), height);
    }
}
