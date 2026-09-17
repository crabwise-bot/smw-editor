//! Lunar Magic v3.00 per-level sprite-header options beyond the vanilla byte.
//!
//! LM v3.00's "Change Properties in Sprite Header" dialog added two controls
//! on top of the vanilla header byte (sprite memory / buoyancy / Layer 2
//! interaction, see [`crate::level::headers::SpriteHeader`]):
//!
//! * **sprite vertical spawning range** — how far above/below the visible
//!   screen (in horizontal levels) a sprite may be and still spawn;
//! * **smart spawning** — LM's improved spawn logic.
//!
//! Both arrived together with LM v3.00's taller-level support (same version
//! as the dynamic level heights): in levels taller than vanilla, the stock
//! spawn logic misbehaves, so these are the tuning knobs for it.
//!
//! # Storage format (smw-editor native, documented)
//!
//! There is no vanilla ROM structure for these settings — LM implements them
//! as an ASM patch plus its own per-level storage — so smw-editor stores them
//! in a single RATS-tagged free-space block, exactly like
//! [`crate::level::dimensions::LevelHeights`] (the LM v3.00 level-height
//! parity slice, PR #34):
//!
//! ```text
//! "SMWSPRH1"            8 bytes magic
//! version               u8 (=1)
//! level_count           u16 LE
//! per level entry:
//!   level               u16 LE (0x000-0x1FF)
//!   spawn_range         u8 (0=Normal, 1=Wide, 2=Wider, 3=Widest)
//!   smart_spawning      u8 (0/1)
//! ```
//!
//! Only non-default entries are stored (Normal range + smart spawning off is
//! the implicit default), so a ROM that never touches these options carries
//! no block at all. The RATS tag is the standard `STAR` + size + ~size header
//! LM itself uses, so other tools' free-space scanners won't clobber it.
//!
//! # Honest boundary
//!
//! smw-editor does **not** install LM's in-game sprite engine: on a stock ROM
//! these two settings are inert metadata describing the author's intent (for
//! LM 3.00+ to consume). The vanilla header-byte fields (sprite memory,
//! buoyancy, Layer 2 interaction) are real and take effect on any ROM. The
//! editor UI says so next to the controls.

use std::collections::BTreeMap;

use thiserror::Error;

/// Lunar Magic v3.00 raised the per-level sprite cap from 84 to 128 sprites
/// for non-SA1 ROMs (official readme). smw-editor's sprite list is an
/// unbounded `Vec` (no 84-cap to raise in code); this constant documents the
/// LM limit the editor UI warns about.
pub const MAX_SPRITES_LM300: usize = 128;

/// Magic at the start of the RATS payload.
pub const SPRITE_HEADER_EXT_MAGIC: &[u8; 8] = b"SMWSPRH1";

/// Current payload format version.
pub const SPRITE_HEADER_EXT_VERSION: u8 = 1;

/// Vanilla vertical spawn margin, in pixels: sprites spawn while within
/// about one tile of the visible screen edge vertically.
pub const VANILLA_SPAWN_MARGIN_PX: u32 = 16;

#[derive(Debug, Error)]
pub enum SpriteHeaderExtError {
    #[error("no sprite-header-ext block in ROM")]
    NotFound,
    #[error("sprite-header-ext block is corrupt: {0}")]
    Corrupt(String),
    #[error("level {0:#05X} out of range (max 0x1FF)")]
    BadLevel(u16),
    #[error("spawn_range {0} out of range 0..=3")]
    BadSpawnRange(u8),
    #[error("sprite-header-ext payload too large for a RATS block ({0} bytes)")]
    TooLarge(usize),
    #[error("no free space for sprite-header-ext block ({0} bytes)")]
    NoFreeSpace(usize),
}

/// Sprite vertical spawning range for horizontal levels (LM v3.00).
///
/// The vanilla game spawns sprites while they are within roughly
/// [`VANILLA_SPAWN_MARGIN_PX`] of the screen's top/bottom edge; the wider
/// settings extend that window for tall levels where the vanilla margin
/// starves sprites of spawns. Exact in-game margins come from LM's inserted
/// sprite engine; smw-editor records the author's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SpawnRange {
    /// Vanilla behavior (~16 px margin).
    #[default]
    Normal = 0,
    /// ~32 px margin.
    Wide   = 1,
    /// ~48 px margin.
    Wider  = 2,
    /// ~64 px margin.
    Widest = 3,
}

impl SpawnRange {
    /// All options in dropdown order.
    pub const ALL: [SpawnRange; 4] = [SpawnRange::Normal, SpawnRange::Wide, SpawnRange::Wider, SpawnRange::Widest];

    /// Decode a stored byte; unknown values are rejected (not coerced).
    pub fn from_byte(b: u8) -> Result<Self, SpriteHeaderExtError> {
        match b {
            0 => Ok(SpawnRange::Normal),
            1 => Ok(SpawnRange::Wide),
            2 => Ok(SpawnRange::Wider),
            3 => Ok(SpawnRange::Widest),
            other => Err(SpriteHeaderExtError::BadSpawnRange(other)),
        }
    }

    /// UI label, matching the dialog's wording.
    pub fn label(self) -> &'static str {
        match self {
            SpawnRange::Normal => "Normal (vanilla ~16 px)",
            SpawnRange::Wide => "Wide (~32 px)",
            SpawnRange::Wider => "Wider (~48 px)",
            SpawnRange::Widest => "Widest (~64 px)",
        }
    }
}

/// LM v3.00 per-level sprite-header options for one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpriteHeaderExt {
    pub spawn_range:    SpawnRange,
    pub smart_spawning: bool,
}

impl SpriteHeaderExt {
    /// Whether this is the vanilla default (not worth storing).
    pub fn is_default(self) -> bool {
        self.spawn_range == SpawnRange::Normal && !self.smart_spawning
    }
}

/// Per-level LM v3.00 sprite-header options.
///
/// Levels not present in the map use [`SpriteHeaderExt::default`].
#[derive(Debug, Clone, Default)]
pub struct SpriteHeaderExtData {
    entries: BTreeMap<u16, SpriteHeaderExt>,
}

impl SpriteHeaderExtData {
    /// Options for `level` (vanilla defaults when unset).
    pub fn get(&self, level: u16) -> SpriteHeaderExt {
        self.entries.get(&level).copied().unwrap_or_default()
    }

    /// Whether `level` has non-default options stored.
    pub fn is_custom(&self, level: u16) -> bool {
        self.entries.contains_key(&level)
    }

    /// Set `level`'s options. Setting the default clears the entry instead
    /// of storing it.
    pub fn set(&mut self, level: u16, ext: SpriteHeaderExt) -> Result<(), SpriteHeaderExtError> {
        if level >= 0x200 {
            return Err(SpriteHeaderExtError::BadLevel(level));
        }
        if ext.is_default() {
            self.entries.remove(&level);
        } else {
            self.entries.insert(level, ext);
        }
        Ok(())
    }

    /// Remove any stored options for `level`.
    pub fn clear(&mut self, level: u16) {
        self.entries.remove(&level);
    }

    /// Number of levels with non-default options.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no options are stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over `(level, options)` pairs in level order.
    pub fn iter(&self) -> impl Iterator<Item = (u16, SpriteHeaderExt)> + '_ {
        self.entries.iter().map(|(&l, &e)| (l, e))
    }

    /// Parse the sprite-header-ext block from raw ROM bytes. Returns
    /// [`SpriteHeaderExtError::NotFound`] when no block exists yet.
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, SpriteHeaderExtError> {
        let tag = find_block(rom_bytes).ok_or(SpriteHeaderExtError::NotFound)?;
        let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
        let end = tag.saturating_add(8).saturating_add(size).saturating_add(1);
        let payload = rom_bytes
            .get(tag + 8..end)
            .ok_or_else(|| SpriteHeaderExtError::Corrupt("sprite-header-ext block overruns ROM".into()))?;
        decode_payload(payload)
    }

    /// Write the data to ROM: erase any existing block (fill with `0xFF` so
    /// it reads as free space again), allocate fresh free space, and write a
    /// new RATS-tagged block. Empty data erases the block without writing a
    /// new one.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), SpriteHeaderExtError> {
        if let Some(tag) = find_block(rom_bytes) {
            let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }

        if self.entries.is_empty() {
            return Ok(());
        }

        let payload = encode_payload(self)?;
        let total = 8 + payload.len(); // RATS tag + payload
        let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
            .ok_or(SpriteHeaderExtError::NoFreeSpace(total))?;
        let file_off = pc + header_offset;
        if payload.len() > 0x10000 {
            return Err(SpriteHeaderExtError::TooLarge(payload.len()));
        }
        let size_field = (payload.len() - 1) as u16;
        rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
        rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
        rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
        rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);
        Ok(())
    }
}

fn encode_payload(data: &SpriteHeaderExtData) -> Result<Vec<u8>, SpriteHeaderExtError> {
    let mut out = Vec::with_capacity(11 + data.entries.len() * 4);
    out.extend_from_slice(SPRITE_HEADER_EXT_MAGIC);
    out.push(SPRITE_HEADER_EXT_VERSION);
    out.extend_from_slice(&(data.entries.len() as u16).to_le_bytes());
    for (&level, ext) in &data.entries {
        out.extend_from_slice(&level.to_le_bytes());
        out.push(ext.spawn_range as u8);
        out.push(u8::from(ext.smart_spawning));
    }
    if out.len() > 0x10001 {
        return Err(SpriteHeaderExtError::TooLarge(out.len()));
    }
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<SpriteHeaderExtData, SpriteHeaderExtError> {
    let corrupt = |msg: String| SpriteHeaderExtError::Corrupt(msg);
    if payload.len() < 11 {
        return Err(corrupt("payload shorter than header".into()));
    }
    if &payload[..8] != SPRITE_HEADER_EXT_MAGIC {
        return Err(corrupt("bad magic".into()));
    }
    if payload[8] != SPRITE_HEADER_EXT_VERSION {
        return Err(corrupt(format!("unsupported version {}", payload[8])));
    }
    let count = u16::from_le_bytes([payload[9], payload[10]]) as usize;
    if payload.len() != 11 + count * 4 {
        return Err(corrupt("payload length does not match level count".into()));
    }
    let mut entries = BTreeMap::new();
    for i in 0..count {
        let off = 11 + i * 4;
        let level = u16::from_le_bytes([payload[off], payload[off + 1]]);
        if level >= 0x200 {
            return Err(corrupt("level number out of range".into()));
        }
        let spawn_range = SpawnRange::from_byte(payload[off + 2]).map_err(|e| corrupt(e.to_string()))?;
        let smart = match payload[off + 3] {
            0 => false,
            1 => true,
            other => return Err(corrupt(format!("smart_spawning byte {other} not 0/1"))),
        };
        let ext = SpriteHeaderExt { spawn_range, smart_spawning: smart };
        if ext.is_default() {
            return Err(corrupt("default entry should not be stored".into()));
        }
        if entries.insert(level, ext).is_some() {
            return Err(corrupt("duplicate level entry".into()));
        }
    }
    Ok(SpriteHeaderExtData { entries })
}

/// Scan `rom_bytes` for the sprite-header-ext RATS block. Returns the file
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
                    && &rom_bytes[data_start..data_start + 8] == SPRITE_HEADER_EXT_MAGIC
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
    fn spawn_range_round_trips_bytes() {
        for (b, range) in
            [(0u8, SpawnRange::Normal), (1, SpawnRange::Wide), (2, SpawnRange::Wider), (3, SpawnRange::Widest)]
        {
            assert_eq!(SpawnRange::from_byte(b).unwrap(), range);
            assert_eq!(range as u8, b);
        }
        assert!(SpawnRange::from_byte(4).is_err());
        assert!(SpawnRange::from_byte(0xFF).is_err());
    }

    #[test]
    fn default_options_are_not_stored() {
        let mut data = SpriteHeaderExtData::default();
        assert!(data.is_empty());
        // Setting the default is a no-op / clears.
        data.set(0x105, SpriteHeaderExt::default()).unwrap();
        assert!(data.is_empty());
        assert_eq!(data.get(0x105), SpriteHeaderExt::default());
        assert!(!data.is_custom(0x105));

        data.set(0x105, SpriteHeaderExt { spawn_range: SpawnRange::Wider, smart_spawning: true }).unwrap();
        assert!(data.is_custom(0x105));
        assert_eq!(data.len(), 1);
        // Back to default clears the entry.
        data.set(0x105, SpriteHeaderExt::default()).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn bad_level_rejected() {
        let mut data = SpriteHeaderExtData::default();
        let ext = SpriteHeaderExt { spawn_range: SpawnRange::Wide, smart_spawning: false };
        assert!(data.set(0x200, ext).is_err());
        assert!(data.set(0x1FF, ext).is_ok());
    }

    #[test]
    fn payload_round_trips() {
        let mut data = SpriteHeaderExtData::default();
        data.set(0x007, SpriteHeaderExt { spawn_range: SpawnRange::Widest, smart_spawning: true }).unwrap();
        data.set(0x105, SpriteHeaderExt { spawn_range: SpawnRange::Wide, smart_spawning: false }).unwrap();
        let payload = encode_payload(&data).unwrap();
        let back = decode_payload(&payload).unwrap();
        assert_eq!(back.get(0x007).spawn_range, SpawnRange::Widest);
        assert!(back.get(0x007).smart_spawning);
        assert_eq!(back.get(0x105).spawn_range, SpawnRange::Wide);
        assert!(!back.get(0x105).smart_spawning);
        assert_eq!(back.get(0x000), SpriteHeaderExt::default());
        // Level order is stable (BTreeMap).
        let levels: Vec<u16> = back.iter().map(|(l, _)| l).collect();
        assert_eq!(levels, vec![0x007, 0x105]);
    }

    #[test]
    fn corrupt_payloads_rejected() {
        let mut data = SpriteHeaderExtData::default();
        data.set(0x105, SpriteHeaderExt { spawn_range: SpawnRange::Wide, smart_spawning: true }).unwrap();
        let mut payload = encode_payload(&data).unwrap();
        // Bad magic.
        let mut bad = payload.clone();
        bad[0] = b'X';
        assert!(decode_payload(&bad).is_err());
        // Bad version.
        let mut bad = payload.clone();
        bad[8] = 0xFF;
        assert!(decode_payload(&bad).is_err());
        // Truncated.
        assert!(decode_payload(&payload[..payload.len() - 1]).is_err());
        assert!(decode_payload(&payload[..5]).is_err());
        // Bad spawn_range byte.
        payload[11 + 2] = 9;
        assert!(decode_payload(&payload).is_err());
    }

    #[test]
    fn write_and_parse_round_trip_on_scratch() {
        // Scratch "ROM": mostly 0xFF free space. find_free_space() only
        // considers PC addresses >= 0x8000, so the buffer must extend past
        // that.
        let mut bytes = vec![0xFFu8; 0x10000];
        let mut data = SpriteHeaderExtData::default();
        data.set(0x105, SpriteHeaderExt { spawn_range: SpawnRange::Wider, smart_spawning: true }).unwrap();
        data.write_to_rom(&mut bytes, 0).unwrap();
        // RATS tag present and parseable.
        let back = SpriteHeaderExtData::parse(&bytes).unwrap();
        assert_eq!(back.get(0x105).spawn_range, SpawnRange::Wider);
        assert!(back.get(0x105).smart_spawning);
        // Writing empty data erases the block.
        let empty = SpriteHeaderExtData::default();
        empty.write_to_rom(&mut bytes, 0).unwrap();
        assert!(matches!(SpriteHeaderExtData::parse(&bytes), Err(SpriteHeaderExtError::NotFound)));
        assert!(bytes.iter().all(|&b| b == 0xFF));
    }

    /// Real-ROM test: a vanilla ROM carries no sprite-header-ext block, and
    /// the block write/parse round-trips on a scratch copy of the real ROM.
    /// Needs `ROM_PATH` pointing at a headerless SMW ROM.
    #[test]
    #[ignore]
    fn real_rom_block_round_trip() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a headerless SMW ROM");
        let mut bytes = std::fs::read(&path).expect("read ROM");
        // Vanilla ROM has no block.
        assert!(matches!(SpriteHeaderExtData::parse(&bytes), Err(SpriteHeaderExtError::NotFound)));

        let mut data = SpriteHeaderExtData::default();
        data.set(0x105, SpriteHeaderExt { spawn_range: SpawnRange::Wide, smart_spawning: true }).unwrap();
        data.write_to_rom(&mut bytes, 0).unwrap();
        let back = SpriteHeaderExtData::parse(&bytes).unwrap();
        assert_eq!(back.get(0x105).spawn_range, SpawnRange::Wide);
        assert!(back.get(0x105).smart_spawning);
        // Untouched levels still read default.
        assert_eq!(back.get(0x000), SpriteHeaderExt::default());
    }

    /// Real-ROM test: the vanilla sprite header byte's bit usage matches the
    /// layout documented on [`crate::level::headers::SpriteHeader`] — bit 5
    /// is never set, and sprite-memory values stay within 0x00-0x12.
    #[test]
    #[ignore]
    fn real_rom_sprite_header_bits_match_documented_layout() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a headerless SMW ROM");
        let rom = crate::SmwRom::from_file(&path).expect("parse ROM");
        for (i, level) in rom.levels.iter().enumerate() {
            let b = level.sprite_header.as_byte();
            assert_eq!(b & 0b00100000, 0, "level {i:03X}: bit 5 set in vanilla sprite header ({b:#04X})");
            assert!(
                level.sprite_header.sprite_memory() <= 0x12,
                "level {i:03X}: sprite memory {:#04X} outside vanilla 0x00-0x12",
                level.sprite_header.sprite_memory()
            );
        }
    }
}
