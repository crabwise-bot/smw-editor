//! Palette file interchange: shared-palette extract/insert and `.mw3`
//! custom-palette files (Lunar Magic's Palette Editor file buttons).
//!
//! Two formats:
//!
//! - **Shared palette extract/insert.** The three shared palette table
//!   groups the palette editor manages — BG (`$00B0B0`), FG (`$00B190`)
//!   and sprite (`$00B318`), 8 rows of 12 SNES colors each — serialized as
//!   a flat 576-byte little-endian u16 sequence (BG rows, then FG rows,
//!   then sprite rows). The bytes are identical to the ROM ranges, so an
//!   extract→insert round-trip restores the tables exactly. (Lunar Magic's
//!   own shared-palette file layout could not be verified offline, so this
//!   format is editor-defined and documented as such; it is at least
//!   lossless for this editor.)
//! - **`.mw3` custom-palette files** (Lunar Magic v1.40, File menu:
//!   "export and import custom level palette files (MW3)" — FuSoYa's
//!   release notes, 2001-12-29). 514 bytes = 257 little-endian u16 SNES
//!   colors; the exact size was verified against files exported by real
//!   Lunar Magic 3.63. This editor's level palette is the 36 colors of the
//!   level's BG/FG/sprite rows, so words 0..36 carry those (BG, FG,
//!   sprite) and words 36..257 are written as zero; import reads words
//!   0..36 and ignores the rest, so a full 256-color LM `.mw3` contributes
//!   its first 36 words.

use std::fmt;

/// Rows per shared palette table group (BG, FG, sprite).
pub const SHARED_ROWS_PER_GROUP: usize = 8;
/// SNES colors per palette row.
pub const COLORS_PER_ROW: usize = 12;
/// Total shared-palette colors across the three groups (8 rows × 12 × 3).
pub const SHARED_PALETTE_COLORS: usize = SHARED_ROWS_PER_GROUP * COLORS_PER_ROW * 3;
/// Serialized bytes of a shared-palette file (288 little-endian u16).
pub const SHARED_PALETTE_BYTES: usize = SHARED_PALETTE_COLORS * 2;
/// Words in a `.mw3` custom-palette file.
pub const MW3_WORDS: usize = 257;
/// Serialized bytes of a `.mw3` file (257 little-endian u16).
pub const MW3_BYTES: usize = MW3_WORDS * 2;
/// `.mw3` words carrying this editor's 36 level colors (12 BG + 12 FG + 12
/// sprite); the remaining words are zero on export and ignored on import.
pub const MW3_LEVEL_COLORS: usize = 36;

/// Errors from the palette file parsers. Both parsers are strict about
/// size: anything that is not exactly the expected byte count is rejected,
/// so a truncated or foreign file can never half-apply (failure-atomic
/// imports).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteFileError {
    BadSharedSize { expected: usize, got: usize },
    BadMw3Size { expected: usize, got: usize },
}

impl fmt::Display for PaletteFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PaletteFileError::BadSharedSize { expected, got } => {
                write!(f, "shared-palette file must be exactly {expected} bytes, got {got}")
            }
            PaletteFileError::BadMw3Size { expected, got } => {
                write!(f, ".mw3 file must be exactly {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for PaletteFileError {}

/// The three shared palette table groups the palette editor manages: BG
/// (`$00B0B0`), FG (`$00B190`) and sprite (`$00B318`), 8 rows of 12 SNES
/// ABGR1555 colors each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SharedPaletteTables {
    pub bg:     [[u16; COLORS_PER_ROW]; SHARED_ROWS_PER_GROUP],
    pub fg:     [[u16; COLORS_PER_ROW]; SHARED_ROWS_PER_GROUP],
    pub sprite: [[u16; COLORS_PER_ROW]; SHARED_ROWS_PER_GROUP],
}

impl SharedPaletteTables {
    /// Serialize to the extract file format: BG rows, then FG rows, then
    /// sprite rows, each color a little-endian u16 — byte-identical to the
    /// ROM ranges.
    pub fn to_bytes(&self) -> [u8; SHARED_PALETTE_BYTES] {
        let mut out = [0u8; SHARED_PALETTE_BYTES];
        let mut o = 0;
        for row in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            for &c in row.iter() {
                out[o..o + 2].copy_from_slice(&c.to_le_bytes());
                o += 2;
            }
        }
        out
    }

    /// Strict parse of an extracted shared-palette file: anything that is
    /// not exactly 576 bytes is rejected, so a truncated or foreign file
    /// can never half-apply.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PaletteFileError> {
        if bytes.len() != SHARED_PALETTE_BYTES {
            return Err(PaletteFileError::BadSharedSize { expected: SHARED_PALETTE_BYTES, got: bytes.len() });
        }
        let mut tables = Self::default();
        let mut words = bytes.chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]]));
        for row in tables.bg.iter_mut().chain(tables.fg.iter_mut()).chain(tables.sprite.iter_mut()) {
            for c in row.iter_mut() {
                *c = words.next().expect("chunk count matches SHARED_PALETTE_COLORS");
            }
        }
        Ok(tables)
    }

    /// Row `0..24` across the three groups (0–7 BG, 8–15 FG, 16–23 sprite).
    pub fn row(&self, row: usize) -> &[u16; COLORS_PER_ROW] {
        match row / SHARED_ROWS_PER_GROUP {
            0 => &self.bg[row % SHARED_ROWS_PER_GROUP],
            1 => &self.fg[row % SHARED_ROWS_PER_GROUP],
            _ => &self.sprite[row % SHARED_ROWS_PER_GROUP],
        }
    }

    /// Mutable row `0..24` across the three groups.
    pub fn row_mut(&mut self, row: usize) -> &mut [u16; COLORS_PER_ROW] {
        match row / SHARED_ROWS_PER_GROUP {
            0 => &mut self.bg[row % SHARED_ROWS_PER_GROUP],
            1 => &mut self.fg[row % SHARED_ROWS_PER_GROUP],
            _ => &mut self.sprite[row % SHARED_ROWS_PER_GROUP],
        }
    }
}

/// One level's 36 palette colors (BG/FG/sprite rows) — the unit `.mw3`
/// files carry for this editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelPalette36 {
    pub bg:     [u16; COLORS_PER_ROW],
    pub fg:     [u16; COLORS_PER_ROW],
    pub sprite: [u16; COLORS_PER_ROW],
}

/// Write a `.mw3` custom-palette file: 257 little-endian u16 SNES colors.
/// Words 0..36 are the level's BG/FG/sprite colors; the rest are zero (see
/// the module docs for why).
pub fn write_mw3(pal: &LevelPalette36) -> [u8; MW3_BYTES] {
    let mut out = [0u8; MW3_BYTES];
    let mut o = 0;
    for &c in pal.bg.iter().chain(pal.fg.iter()).chain(pal.sprite.iter()) {
        out[o..o + 2].copy_from_slice(&c.to_le_bytes());
        o += 2;
    }
    out
}

/// Strict parse of a `.mw3` file: exactly 514 bytes, else rejected. Reads
/// words 0..36 into the level palette and ignores the rest.
pub fn read_mw3(bytes: &[u8]) -> Result<LevelPalette36, PaletteFileError> {
    if bytes.len() != MW3_BYTES {
        return Err(PaletteFileError::BadMw3Size { expected: MW3_BYTES, got: bytes.len() });
    }
    let mut pal = LevelPalette36::default();
    let mut words = bytes.chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).take(MW3_LEVEL_COLORS);
    for slot in pal.bg.iter_mut().chain(pal.fg.iter_mut()).chain(pal.sprite.iter_mut()) {
        *slot = words.next().expect("take(MW3_LEVEL_COLORS) yields 36 words");
    }
    Ok(pal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tables() -> SharedPaletteTables {
        let mut t = SharedPaletteTables::default();
        for (r, row) in t.bg.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                *slot = (r * 16 + c) as u16;
            }
        }
        for (r, row) in t.fg.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                *slot = 0x1000 + (r * 16 + c) as u16;
            }
        }
        for (r, row) in t.sprite.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                *slot = 0x2000 + (r * 16 + c) as u16;
            }
        }
        t
    }

    #[test]
    fn shared_palette_round_trips_exactly() {
        let t = sample_tables();
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), SHARED_PALETTE_BYTES);
        assert_eq!(SharedPaletteTables::from_bytes(&bytes).unwrap(), t);
    }

    #[test]
    fn shared_palette_layout_is_bg_then_fg_then_sprite() {
        let t = sample_tables();
        let bytes = t.to_bytes();
        // First BG row, first color = 0x0000; first FG row starts at byte 192.
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 0x0000);
        assert_eq!(u16::from_le_bytes([bytes[192], bytes[193]]), 0x1000);
        // First sprite row starts at byte 384.
        assert_eq!(u16::from_le_bytes([bytes[384], bytes[385]]), 0x2000);
        // Last word is sprite row 7, color 11.
        let last = u16::from_le_bytes([bytes[574], bytes[575]]);
        assert_eq!(last, 0x2000 + 7 * 16 + 11);
    }

    #[test]
    fn shared_palette_rejects_wrong_sizes() {
        for len in [0, 1, 575, 577, 1024] {
            let bytes = vec![0u8; len];
            assert_eq!(
                SharedPaletteTables::from_bytes(&bytes),
                Err(PaletteFileError::BadSharedSize { expected: SHARED_PALETTE_BYTES, got: len }),
                "must reject {len} bytes"
            );
        }
    }

    #[test]
    fn mw3_has_lm_size_and_zero_tail() {
        let pal = LevelPalette36 { bg: [0x001F; 12], fg: [0x03E0; 12], sprite: [0x7C00; 12] };
        let bytes = write_mw3(&pal);
        assert_eq!(bytes.len(), MW3_BYTES);
        assert_eq!(bytes.len(), 514, ".mw3 must be exactly 514 bytes like Lunar Magic's");
        // Words 0..36 carry the colors…
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 0x001F);
        assert_eq!(u16::from_le_bytes([bytes[24], bytes[25]]), 0x03E0);
        assert_eq!(u16::from_le_bytes([bytes[48], bytes[49]]), 0x7C00);
        // …words 36..257 are zero.
        assert!(bytes[72..].iter().all(|&b| b == 0));
    }

    #[test]
    fn mw3_round_trips_level_colors() {
        let pal = LevelPalette36 { bg: [0x1234; 12], fg: [0x5678; 12], sprite: [0x7FFF; 12] };
        let back = read_mw3(&write_mw3(&pal)).unwrap();
        assert_eq!(back, pal);
    }

    #[test]
    fn mw3_import_reads_first_36_words_of_full_lm_file() {
        // A full 256-color LM .mw3: only the first 36 words matter to us.
        let mut bytes = [0u8; MW3_BYTES];
        for i in 0..MW3_WORDS {
            bytes[i * 2..i * 2 + 2].copy_from_slice(&(0x4000 + i as u16).to_le_bytes());
        }
        let pal = read_mw3(&bytes).unwrap();
        assert_eq!(pal.bg[0], 0x4000);
        assert_eq!(pal.bg[11], 0x400B);
        assert_eq!(pal.fg[0], 0x400C);
        assert_eq!(pal.sprite[11], 0x4023);
    }

    #[test]
    fn mw3_rejects_wrong_sizes() {
        for len in [0, 72, 513, 515, 576] {
            let bytes = vec![0u8; len];
            assert_eq!(
                read_mw3(&bytes),
                Err(PaletteFileError::BadMw3Size { expected: MW3_BYTES, got: len }),
                "must reject {len} bytes"
            );
        }
    }

    #[test]
    fn shared_row_accessors_span_all_24_rows() {
        let mut t = SharedPaletteTables::default();
        *t.row_mut(0) = [1; 12];
        *t.row_mut(8) = [2; 12];
        *t.row_mut(16) = [3; 12];
        *t.row_mut(23) = [4; 12];
        assert_eq!(t.row(0), &[1; 12]);
        assert_eq!(t.row(8), &[2; 12]);
        assert_eq!(t.row(16), &[3; 12]);
        assert_eq!(t.row(23), &[4; 12]);
        assert_eq!(t.bg[0], [1; 12]);
        assert_eq!(t.fg[0], [2; 12]);
        assert_eq!(t.sprite[7], [4; 12]);
    }
}
