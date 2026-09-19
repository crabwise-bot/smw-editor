//! Per-level custom palettes (Lunar Magic v3.30: "Auto-Enable custom palette on
//! edit").
//!
//! In vanilla SMW a level's BG/FG/sprite palettes are indices into the game's
//! shared palette tables, so editing one level's colors changes every other
//! level that shares the table entry. Lunar Magic's "Enable Custom Palette"
//! gives a level its own private copy instead, and v3.30 added an option to
//! enable it automatically the moment a palette edit is made.
//!
//! This editor cannot install LM's in-game custom-palette ASM (like the other
//! editor-native features, in-game playback of a custom palette needs Lunar
//! Magic), so the per-level palettes live in one editor-owned RATS block
//! (`SMWECPLT`), exactly like the secondary-exit extended data
//! (`SMWESEX2`), the dynamic level heights (`SMWLVLH1`), and the Direct Map16
//! data (`SMWDM161`). A ROM nobody has enabled a custom palette for simply
//! has no block, and untouched ROMs stay byte-identical.

use std::collections::BTreeMap;

use thiserror::Error;

use super::LEVEL_COUNT;

/// Magic at the start of the RATS payload.
pub const CUSTOM_PALETTE_MAGIC: &[u8; 8] = b"SMWECPLT";

/// Current payload format version.
pub const CUSTOM_PALETTE_VERSION: u8 = 1;

/// Colors per custom palette: 12 BG + 12 FG + 12 sprite.
pub const CUSTOM_PALETTE_COLORS: usize = 36;

/// Serialized bytes per palette entry (36 little-endian u16 colors).
pub const CUSTOM_PALETTE_ENTRY_BYTES: usize = CUSTOM_PALETTE_COLORS * 2;

#[derive(Debug, Error)]
pub enum CustomPaletteError {
    #[error("no custom-palette block in ROM")]
    NotFound,
    #[error("custom-palette block is corrupt: {0}")]
    Corrupt(String),
    #[error("custom-palette payload too large for a RATS block ({0} bytes)")]
    TooLarge(usize),
    #[error("no free space for custom-palette block ({0} bytes)")]
    NoFreeSpace(usize),
}

/// One level's private BG/FG/sprite palettes (12 colors each, SNES ABGR1555).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomPalette {
    pub bg:     [u16; 12],
    pub fg:     [u16; 12],
    pub sprite: [u16; 12],
}

impl CustomPalette {
    fn to_bytes(&self) -> [u8; CUSTOM_PALETTE_ENTRY_BYTES] {
        let mut out = [0u8; CUSTOM_PALETTE_ENTRY_BYTES];
        for (i, &c) in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()).enumerate() {
            out[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        out
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != CUSTOM_PALETTE_ENTRY_BYTES {
            return None;
        }
        let mut pal = Self::default();
        for (i, chunk) in bytes.chunks_exact(2).enumerate() {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            let slot = match i / 12 {
                0 => &mut pal.bg,
                1 => &mut pal.fg,
                _ => &mut pal.sprite,
            };
            slot[i % 12] = v;
        }
        Some(pal)
    }
}

/// Per-level custom palettes, keyed by level number `0..0x200`.
///
/// Levels not present in the map use the shared palette tables (the vanilla
/// behavior), exactly like a level whose "Enable Custom Palette" is off in
/// Lunar Magic.
#[derive(Clone, Debug, Default)]
pub struct CustomPaletteData {
    palettes: BTreeMap<u16, CustomPalette>,
}

impl CustomPaletteData {
    /// Custom palette for `level`, or `None` when the level uses the shared
    /// tables.
    pub fn get(&self, level: u16) -> Option<&CustomPalette> {
        self.palettes.get(&level)
    }

    /// Store (or replace) the custom palette for `level`.
    pub fn set(&mut self, level: u16, palette: CustomPalette) {
        self.palettes.insert(level, palette);
    }

    /// Remove any custom palette for `level` (back to the shared tables).
    pub fn clear(&mut self, level: u16) {
        self.palettes.remove(&level);
    }

    /// Number of levels with a custom palette.
    pub fn len(&self) -> usize {
        self.palettes.len()
    }

    /// Whether no custom palettes are stored.
    pub fn is_empty(&self) -> bool {
        self.palettes.is_empty()
    }

    /// Iterate over `(level, palette)` pairs in level order.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &CustomPalette)> + '_ {
        self.palettes.iter().map(|(&l, p)| (l, p))
    }

    /// Parse the custom-palette block from raw ROM bytes. Returns
    /// [`CustomPaletteError::NotFound`] when no block exists yet (a fresh
    /// ROM).
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, CustomPaletteError> {
        let tag = find_block(rom_bytes).ok_or(CustomPaletteError::NotFound)?;
        let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
        let end = tag.saturating_add(8).saturating_add(size).saturating_add(1);
        let payload = rom_bytes
            .get(tag + 8..end)
            .ok_or_else(|| CustomPaletteError::Corrupt("custom-palette block overruns ROM".into()))?;
        decode_payload(payload)
    }

    /// Write the data to ROM: erase any existing block (fill with `0xFF` so
    /// it reads as free space again), allocate fresh free space, and write a
    /// new RATS-tagged block. Empty data erases the block without writing a
    /// new one, so untouched ROMs stay byte-identical.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), CustomPaletteError> {
        if let Some(tag) = find_block(rom_bytes) {
            let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }

        if self.palettes.is_empty() {
            return Ok(());
        }

        let payload = encode_payload(self)?;
        let total = 8 + payload.len(); // RATS tag + payload
        let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
            .ok_or(CustomPaletteError::NoFreeSpace(total))?;
        let file_off = pc + header_offset;
        if payload.len() > 0x10000 {
            return Err(CustomPaletteError::TooLarge(payload.len()));
        }
        let size_field = (payload.len() - 1) as u16;
        rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
        rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
        rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
        rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);
        Ok(())
    }
}

fn encode_payload(data: &CustomPaletteData) -> Result<Vec<u8>, CustomPaletteError> {
    // 8 magic + 1 version + 2 count, then per entry: u16 level + 72 color bytes.
    let mut out = Vec::with_capacity(11 + data.palettes.len() * (2 + CUSTOM_PALETTE_ENTRY_BYTES));
    out.extend_from_slice(CUSTOM_PALETTE_MAGIC);
    out.push(CUSTOM_PALETTE_VERSION);
    out.extend_from_slice(&(data.palettes.len() as u16).to_le_bytes());
    for (&level, palette) in &data.palettes {
        out.extend_from_slice(&level.to_le_bytes());
        out.extend_from_slice(&palette.to_bytes());
    }
    if out.len() > 0x10001 {
        return Err(CustomPaletteError::TooLarge(out.len()));
    }
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<CustomPaletteData, CustomPaletteError> {
    let corrupt = |msg: String| CustomPaletteError::Corrupt(msg);
    if payload.len() < 11 {
        return Err(corrupt("payload shorter than header".into()));
    }
    if &payload[..8] != CUSTOM_PALETTE_MAGIC {
        return Err(corrupt("bad magic".into()));
    }
    if payload[8] != CUSTOM_PALETTE_VERSION {
        return Err(corrupt(format!("unsupported version {}", payload[8])));
    }
    let count = u16::from_le_bytes([payload[9], payload[10]]) as usize;
    let entry_len = 2 + CUSTOM_PALETTE_ENTRY_BYTES;
    if payload.len() != 11 + count * entry_len {
        return Err(corrupt("payload length does not match palette count".into()));
    }
    let mut palettes = BTreeMap::new();
    for i in 0..count {
        let off = 11 + i * entry_len;
        let level = u16::from_le_bytes([payload[off], payload[off + 1]]);
        if level as usize >= LEVEL_COUNT {
            return Err(corrupt(format!("level {level:#05X} out of range")));
        }
        let palette = CustomPalette::from_bytes(&payload[off + 2..off + entry_len])
            .ok_or_else(|| corrupt("palette entry truncated".into()))?;
        if palettes.insert(level, palette).is_some() {
            return Err(corrupt(format!("duplicate palette entry for level {level:#05X}")));
        }
    }
    Ok(CustomPaletteData { palettes })
}

/// Scan `rom_bytes` for the custom-palette RATS block. Returns the file
/// offset of the `STAR` tag. Same shape as the level-height scanner: the
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
                    && &rom_bytes[data_start..data_start + 8] == CUSTOM_PALETTE_MAGIC
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_palette(seed: u16) -> CustomPalette {
        let mut pal = CustomPalette::default();
        for (i, slot) in pal.bg.iter_mut().chain(pal.fg.iter_mut()).chain(pal.sprite.iter_mut()).enumerate() {
            *slot = seed.wrapping_add(i as u16 * 0x111);
        }
        pal
    }

    #[test]
    fn round_trip_preserves_all_palettes() {
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(0x1234));
        data.set(0x007, sample_palette(0xABCD));
        let payload = encode_payload(&data).unwrap();
        let back = decode_payload(&payload).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.get(0x105), data.get(0x105));
        assert_eq!(back.get(0x007), data.get(0x007));
        assert_eq!(back.get(0x100), None);
    }

    #[test]
    fn empty_data_encodes_and_decodes() {
        let data = CustomPaletteData::default();
        assert!(data.is_empty());
        let back = decode_payload(&encode_payload(&data).unwrap()).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn corrupt_payloads_are_rejected() {
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(1));
        let good = encode_payload(&data).unwrap();

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(decode_payload(&bad_magic).is_err());

        let mut bad_version = good.clone();
        bad_version[8] = 0xFF;
        assert!(decode_payload(&bad_version).is_err());

        let mut bad_count = good.clone();
        bad_count[9] = 0x09; // claim 9 entries, only 1 present
        assert!(decode_payload(&bad_count).is_err());

        assert!(decode_payload(&good[..good.len() - 1]).is_err()); // truncated
        assert!(decode_payload(b"short").is_err());

        // Duplicate level entry.
        let entry_len = 2 + CUSTOM_PALETTE_ENTRY_BYTES;
        let mut dup = good.clone();
        dup.extend_from_slice(&good[11..11 + entry_len]);
        dup[9] = 0x02;
        assert!(decode_payload(&dup).is_err());

        // Out-of-range level number.
        let mut bad_level = good.clone();
        bad_level[11] = 0x00;
        bad_level[12] = 0x02; // level 0x200 == LEVEL_COUNT
        assert!(decode_payload(&bad_level).is_err());
    }

    #[test]
    fn set_clear_get_len_iter() {
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(1));
        data.set(0x105, sample_palette(2)); // replace
        assert_eq!(data.len(), 1);
        data.set(0x001, sample_palette(3));
        let levels: Vec<u16> = data.iter().map(|(l, _)| l).collect();
        assert_eq!(levels, vec![0x001, 0x105]); // level order
        data.clear(0x105);
        assert_eq!(data.get(0x105), None);
        assert!(!data.is_empty());
        data.clear(0x001);
        assert!(data.is_empty());
    }

    #[test]
    fn write_erase_round_trip() {
        let mut rom = vec![0xFFu8; 0x80000];
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(0x1234));
        data.write_to_rom(&mut rom, 0).unwrap();
        let back = CustomPaletteData::parse(&rom).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.get(0x105), data.get(0x105));
        // Rewriting erases the old block: the ROM still parses exactly once.
        data.set(0x105, sample_palette(0x5678));
        data.write_to_rom(&mut rom, 0).unwrap();
        let back2 = CustomPaletteData::parse(&rom).unwrap();
        assert_eq!(back2.len(), 1);
        assert_eq!(back2.get(0x105), data.get(0x105));
        // Empty data erases the block entirely: the ROM is back to
        // all-0xFF, i.e. byte-identical to before.
        CustomPaletteData::default().write_to_rom(&mut rom, 0).unwrap();
        assert!(matches!(CustomPaletteData::parse(&rom), Err(CustomPaletteError::NotFound)));
        assert!(rom.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn ignores_unrelated_rats_blocks() {
        let mut rom = vec![0xFFu8; 0x80000];
        // Some other tool's RATS block.
        rom[0x1000..0x1004].copy_from_slice(b"STAR");
        rom[0x1004..0x1006].copy_from_slice(&7u16.to_le_bytes());
        rom[0x1006..0x1008].copy_from_slice(&(!7u16).to_le_bytes());
        rom[0x1008..0x1010].copy_from_slice(b"OTHERMAG");
        assert!(matches!(CustomPaletteData::parse(&rom), Err(CustomPaletteError::NotFound)));
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(1));
        data.write_to_rom(&mut rom, 0).unwrap();
        assert_eq!(CustomPaletteData::parse(&rom).unwrap().get(0x105), data.get(0x105));
    }

    /// Real-ROM test: write a custom palette into a scratch *copy* of the
    /// real ROM (the file itself is never touched), parse it back, and
    /// confirm the whole ROM still parses with the palette visible on
    /// `SmwRom`. A pristine ROM parses as `NotFound` (no block).
    #[test]
    #[ignore]
    fn real_rom_write_parse_round_trip() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM for ignored tests");
        let raw = std::fs::read(path).expect("cannot read ROM");
        let header_offset = if raw.len() % 0x400 == 0x200 { 0x200 } else { 0 };

        // A pristine ROM has no custom-palette block.
        assert!(matches!(CustomPaletteData::parse(&raw), Err(CustomPaletteError::NotFound)));

        let mut rom = raw.clone();
        let mut data = CustomPaletteData::default();
        data.set(0x105, sample_palette(0x7BDE));
        data.write_to_rom(&mut rom, header_offset).unwrap();

        let back = CustomPaletteData::parse(&rom).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.get(0x105), data.get(0x105));

        // The whole ROM still parses, and SmwRom sees the custom palette.
        let parsed = crate::SmwRom::from_rom(crate::snes_utils::rom::Rom::new(rom).unwrap()).unwrap();
        assert_eq!(parsed.custom_palettes.get(0x105), data.get(0x105));
    }
}
