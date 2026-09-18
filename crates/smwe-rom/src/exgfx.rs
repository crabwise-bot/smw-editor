//! True ExGFX support — extra graphics files (0x80+) + Super GFX Bypass.
//!
//! Lunar Magic parity (LM v1.10/v1.60, audit §3): LM manages up to 0xF80 extra
//! graphics files (`ExGFX80.bin` …) with extract/insert, and its "Super GFX
//! Bypass" dialog assigns FG/BG/SP graphics per level from one place — each of
//! the eight upload slots (FG1/FG2/FG3/BG1/SP1/SP2/SP3/SP4) can point at a
//! vanilla GFX file or an ExGFX file instead of the level tileset tables.
//!
//! # Storage format (smw-editor native, documented)
//!
//! There is no vanilla ROM structure for this (LM implements it as an ASM
//! hack), so smw-editor stores the data in RATS-tagged free-space blocks. The
//! RATS tag is the standard `STAR` + size + ~size header LM itself uses, so
//! other tools' free-space scanners won't clobber the blocks.
//!
//! One RATS block per ExGFX file *chunk* (a 32 KiB file cannot sit in a
//! single block — free space never spans a LoROM bank boundary):
//!
//! ```text
//! "SMWEXGFX"   8 bytes magic
//! version       u8 (=1)
//! file_index    u16 LE (0x80..=0xFFF; LM's ExGFX range runs to 0xFFF —
//!               the v2.30 changelog reserves E00-FFF for as-is files)
//! chunk_no      u16 LE (0-based)
//! chunk_count   u16 LE
//! data_len      u16 LE (<= 0x7000)
//! data          data_len bytes of raw 4bpp tile data; the concatenated
//!               chunks are exactly 0x8000 bytes = 0x400 tiles
//! ```
//!
//! ExGFX files are always 4bpp, like LM's `ExGFXnn.bin` files (exactly
//! 0x8000 bytes = 0x400 tiles on insert).
//!
//! One RATS block for the per-level bypass table:
//!
//! ```text
//! "SMWGFXBP"   8 bytes magic
//! version       u8 (=1)
//! level_count   u16 LE
//! per level entry:
//!   level       u16 LE (0x000-0x1FF)
//!   slots       8 x u16 LE, in FG1/FG2/FG3/BG1/SP1/SP2/SP3/SP4 order:
//!                 0xFFFF = default (the level's FG/BG + sprite tileset tables)
//!                 < 0x80 = vanilla GFX file number
//!                 0x80..=0xFFF = ExGFX file index
//! ```
//!
//! # Preview semantics
//!
//! The editor mirrors what LM's ExGFX ASM hack does at level load: after the
//! game's normal GFX upload, each explicitly-assigned slot is overridden —
//! vanilla files via the game's own `UploadGFXFile` routine
//! (`smwe_emu::emu::upload_gfx_file_to_vram`, bit-exact 3bpp→4bpp expansion),
//! ExGFX files by copying the file's first [`BYPASS_SLOT_BYTES`] bytes over
//! the slot's VRAM range (the `CODE_00AA35` ranges for FG/BG, `UploadSpriteGFX`
//! ranges for sprites). The editor applies this right after
//! `decompress_sublevel`, so the level view, the tile picker, and the
//! headless render binaries all see the bypassed graphics — true WYSIWYG.
//!
//! # In-game playback
//!
//! The editor authors, previews, and stores the data; making it run in a real
//! game still requires installing Lunar Magic's ExGFX ASM hack (LM does this
//! itself when you use its ExGFX tools — "insert ExGFX" on a ROM without the
//! hack prompts to install it). This module documents the data so a future
//! installer — or LM — can consume it.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{
    graphics::gfx_file::{GfxFile, Tile, TileFormat},
    objects::{object_gfx_list::OBJECT_SLOT_VRAM_RANGES, sprite_gfx_list::SPRITE_SLOT_VRAM_BASES},
};

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Magic at the start of each ExGFX file RATS payload.
pub const EXGFX_MAGIC: &[u8; 8] = b"SMWEXGFX";
/// Magic at the start of the bypass-table RATS payload.
pub const BYPASS_MAGIC: &[u8; 8] = b"SMWGFXBP";
/// Payload format version for both block kinds.
pub const EXGFX_FORMAT_VERSION: u8 = 1;

/// First ExGFX file index (LM numbers extra files from 0x80).
pub const EXGFX_FIRST_INDEX: u16 = 0x80;
/// Highest ExGFX file index the format supports (0xFFF; LM's ExGFX range
/// runs 0x80-0xFFF — the v2.30 changelog reserves E00-FFF for as-is files).
pub const EXGFX_MAX_INDEX: u16 = 0xFFF;
/// Tiles per ExGFX file on insert (0x8000 raw bytes, like LM's ExGFXnn.bin).
pub const EXGFX_TILES_PER_FILE: usize = 0x400;
/// Raw 4bpp bytes per ExGFX file.
pub const EXGFX_FILE_BYTES: usize = EXGFX_TILES_PER_FILE * 32;
/// Bytes per 4bpp 8x8 tile.
pub const EXGFX_TILE_BYTES: usize = 32;
/// Max raw data bytes per ExGFX chunk block. Free space never spans a LoROM
/// bank boundary, so a whole 32 KiB file can never sit in one block — files
/// are split into chunks that fit comfortably inside a single 0x8000 bank.
pub const EXGFX_CHUNK_BYTES: usize = 0x7000;

/// Bypass slots, in table order: FG1/FG2/FG3/BG1/SP1/SP2/SP3/SP4.
pub const BYPASS_SLOT_COUNT: usize = 8;
/// Slot value meaning "use the level's FG/BG + sprite tileset tables"
/// (i.e. no bypass for this slot — what the game uploaded stands).
pub const BYPASS_DEFAULT: u16 = 0xFFFF;
/// Tiles copied per bypassed slot (each upload slot holds 0x80 tiles).
pub const BYPASS_TILES_PER_SLOT: usize = 0x80;
/// VRAM bytes per bypassed slot.
pub const BYPASS_SLOT_BYTES: usize = BYPASS_TILES_PER_SLOT * EXGFX_TILE_BYTES;

/// LM-style slot labels, in bypass table order.
pub const BYPASS_SLOT_NAMES: [&str; BYPASS_SLOT_COUNT] = ["FG1", "FG2", "FG3", "BG1", "SP1", "SP2", "SP3", "SP4"];

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ExGfxError {
    #[error("ExGFX data not found in ROM")]
    NotFound,
    #[error("Corrupt ExGFX data: {0}")]
    Corrupt(String),
    #[error("ExGFX file index {0:#05X} out of range (0x80-0xFFF)")]
    BadIndex(u16),
    #[error("ExGFX file must be exactly {expected} bytes, got {got}")]
    BadSize { expected: usize, got: usize },
    #[error("No free space for {0} bytes of ExGFX data")]
    NoFreeSpace(usize),
    #[error("ExGFX payload too large ({0} bytes)")]
    TooLarge(usize),
    #[error("Bad bypass level number {0:#05X}")]
    BadLevel(u16),
    #[error("Bad bypass slot {0}")]
    BadSlot(usize),
    #[error("Bad bypass slot value {0:#06X}")]
    BadSlotValue(u16),
}

// -------------------------------------------------------------------------------------------------
// ExGFX files
// -------------------------------------------------------------------------------------------------

/// One extra graphics file: 4bpp tiles plus the raw bytes they decode from
/// (the raw bytes are what gets written back to ROM).
#[derive(Clone, Debug)]
pub struct ExGfxFile {
    pub index: u16,
    pub tiles: Vec<Tile>,
    raw:       Vec<u8>,
}

impl ExGfxFile {
    /// Build from raw 4bpp bytes (exactly [`EXGFX_FILE_BYTES`]).
    pub fn from_raw(index: u16, raw: Vec<u8>) -> Result<Self, ExGfxError> {
        if !(EXGFX_FIRST_INDEX..=EXGFX_MAX_INDEX).contains(&index) {
            return Err(ExGfxError::BadIndex(index));
        }
        if raw.len() != EXGFX_FILE_BYTES {
            return Err(ExGfxError::BadSize { expected: EXGFX_FILE_BYTES, got: raw.len() });
        }
        let tiles = GfxFile::decode_tiles(&raw, TileFormat::Tile4bpp);
        Ok(Self { index, tiles, raw })
    }

    /// Raw 4bpp bytes (exactly [`EXGFX_FILE_BYTES`]).
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Replace the tile data (e.g. from the 8x8 tile editor); the raw bytes
    /// are re-encoded so ROM write-back stays in sync.
    pub fn set_tiles(&mut self, tiles: Vec<Tile>) -> Result<(), ExGfxError> {
        if tiles.len() != EXGFX_TILES_PER_FILE {
            return Err(ExGfxError::BadSize { expected: EXGFX_TILES_PER_FILE, got: tiles.len() });
        }
        self.raw = GfxFile { tile_format: TileFormat::Tile4bpp, tiles: tiles.clone() }.to_raw_bytes();
        debug_assert_eq!(self.raw.len(), EXGFX_FILE_BYTES);
        self.tiles = tiles;
        Ok(())
    }
}

/// All ExGFX files stored in the ROM, keyed by file index.
#[derive(Clone, Debug, Default)]
pub struct ExGfxData {
    pub files: BTreeMap<u16, ExGfxFile>,
}

impl ExGfxData {
    /// Decode one chunk payload into `(index, chunk_no, chunk_count, data)`.
    fn decode_chunk_payload(payload: &[u8]) -> Result<(u16, u16, u16, Vec<u8>), ExGfxError> {
        let err = |m: &str| ExGfxError::Corrupt(m.to_string());
        if payload.len() < 9 {
            return Err(err("ExGFX chunk payload too short"));
        }
        if payload[0] != EXGFX_FORMAT_VERSION {
            return Err(err("unsupported ExGFX payload version"));
        }
        let index = u16::from_le_bytes([payload[1], payload[2]]);
        let chunk_no = u16::from_le_bytes([payload[3], payload[4]]);
        let chunk_count = u16::from_le_bytes([payload[5], payload[6]]);
        let data_len = u16::from_le_bytes([payload[7], payload[8]]) as usize;
        if !(EXGFX_FIRST_INDEX..=EXGFX_MAX_INDEX).contains(&index) {
            return Err(ExGfxError::BadIndex(index));
        }
        if chunk_count == 0 || chunk_no >= chunk_count {
            return Err(err("bad ExGFX chunk numbering"));
        }
        if data_len > EXGFX_CHUNK_BYTES || payload.len() < 9 + data_len {
            return Err(err("ExGFX chunk truncated"));
        }
        Ok((index, chunk_no, chunk_count, payload[9..9 + data_len].to_vec()))
    }

    fn encode_chunk_payload(index: u16, chunk_no: u16, chunk_count: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + data.len());
        out.push(EXGFX_FORMAT_VERSION);
        out.extend_from_slice(&index.to_le_bytes());
        out.extend_from_slice(&chunk_no.to_le_bytes());
        out.extend_from_slice(&chunk_count.to_le_bytes());
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    /// Parse every ExGFX file block from raw ROM bytes (SMC header included
    /// if present — the scan is magic-driven, so the header is harmless).
    /// A file is only returned when all of its chunks are present and
    /// concatenate to exactly [`EXGFX_FILE_BYTES`] bytes.
    pub fn parse(rom_bytes: &[u8]) -> Self {
        let mut chunks: BTreeMap<u16, Vec<(u16, u16, Vec<u8>)>> = BTreeMap::new();
        for tag in find_rats_blocks(rom_bytes, EXGFX_MAGIC) {
            let size = rats_size(rom_bytes, tag);
            // Payload follows the 8-byte magic (same layout as write_rats_block);
            // RATS size field = data_len - 1, hence the +1.
            let payload = &rom_bytes[tag + 16..tag + 8 + size + 1];
            if let Ok((index, chunk_no, chunk_count, data)) = Self::decode_chunk_payload(payload) {
                chunks.entry(index).or_default().push((chunk_no, chunk_count, data));
            }
        }
        let mut files = BTreeMap::new();
        for (index, mut v) in chunks {
            v.sort_by_key(|&(no, _, _)| no);
            let chunk_count = v[0].1 as usize;
            if v.len() != chunk_count
                || v.iter().enumerate().any(|(i, &(no, cc, _))| no as usize != i || cc as usize != chunk_count)
            {
                continue; // incomplete or inconsistent chunk set
            }
            let raw: Vec<u8> = v.into_iter().flat_map(|(_, _, d)| d).collect();
            if raw.len() == EXGFX_FILE_BYTES {
                if let Ok(file) = ExGfxFile::from_raw(index, raw) {
                    files.insert(index, file);
                }
            }
        }
        Self { files }
    }

    /// Insert (or replace) a file from raw 4bpp bytes.
    pub fn insert_raw(&mut self, index: u16, raw: Vec<u8>) -> Result<(), ExGfxError> {
        let file = ExGfxFile::from_raw(index, raw)?;
        self.files.insert(index, file);
        Ok(())
    }

    /// Delete a file. Returns true when a file was actually removed.
    pub fn remove(&mut self, index: u16) -> bool {
        self.files.remove(&index).is_some()
    }

    /// Mutable access to a file's tiles + raw bytes.
    pub fn get_mut(&mut self, index: u16) -> Option<&mut ExGfxFile> {
        self.files.get_mut(&index)
    }

    /// Write every file to ROM: erase all existing ExGFX blocks (fill with
    /// `0xFF` so they read as free space again), then allocate fresh
    /// free space per chunk. Empty data erases the blocks without writing.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), ExGfxError> {
        for tag in find_rats_blocks(rom_bytes, EXGFX_MAGIC) {
            let size = rats_size(rom_bytes, tag);
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }
        for file in self.files.values() {
            let chunk_count = file.raw.len().div_ceil(EXGFX_CHUNK_BYTES) as u16;
            for (chunk_no, chunk) in file.raw.chunks(EXGFX_CHUNK_BYTES).enumerate() {
                let payload = Self::encode_chunk_payload(file.index, chunk_no as u16, chunk_count, chunk);
                write_rats_block(rom_bytes, EXGFX_MAGIC, &payload, header_offset)?;
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// Super GFX Bypass table
// -------------------------------------------------------------------------------------------------

/// Per-level Super GFX Bypass assignments: level number → 8 slot values in
/// FG1/FG2/FG3/BG1/SP1/SP2/SP3/SP4 order.
#[derive(Clone, Debug, Default)]
pub struct BypassData {
    pub levels: BTreeMap<u16, [u16; BYPASS_SLOT_COUNT]>,
}

impl BypassData {
    fn decode_payload(payload: &[u8]) -> Result<Self, ExGfxError> {
        let err = |m: &str| ExGfxError::Corrupt(m.to_string());
        if payload.len() < 3 {
            return Err(err("bypass payload too short"));
        }
        if payload[0] != EXGFX_FORMAT_VERSION {
            return Err(err("unsupported bypass payload version"));
        }
        let count = u16::from_le_bytes([payload[1], payload[2]]) as usize;
        let want = 3 + count * (2 + BYPASS_SLOT_COUNT * 2);
        if payload.len() < want {
            return Err(err("bypass payload truncated"));
        }
        let mut levels = BTreeMap::new();
        let mut off = 3;
        for _ in 0..count {
            let level = u16::from_le_bytes([payload[off], payload[off + 1]]);
            off += 2;
            if level >= 0x200 {
                return Err(ExGfxError::BadLevel(level));
            }
            let mut slots = [BYPASS_DEFAULT; BYPASS_SLOT_COUNT];
            for s in slots.iter_mut() {
                let v = u16::from_le_bytes([payload[off], payload[off + 1]]);
                off += 2;
                if v != BYPASS_DEFAULT && v > EXGFX_MAX_INDEX {
                    return Err(ExGfxError::BadSlotValue(v));
                }
                *s = v;
            }
            levels.insert(level, slots);
        }
        Ok(Self { levels })
    }

    fn encode_payload(&self) -> Result<Vec<u8>, ExGfxError> {
        if self.levels.len() > 0x200 {
            return Err(ExGfxError::Corrupt("too many bypass levels".into()));
        }
        let mut out = Vec::with_capacity(3 + self.levels.len() * 18);
        out.push(EXGFX_FORMAT_VERSION);
        out.extend_from_slice(&(self.levels.len() as u16).to_le_bytes());
        for (&level, slots) in &self.levels {
            out.extend_from_slice(&level.to_le_bytes());
            for &v in slots {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Ok(out)
    }

    /// Parse the bypass table from raw ROM bytes. Returns
    /// [`ExGfxError::NotFound`] when no block exists yet (a fresh ROM).
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, ExGfxError> {
        let tag = find_rats_blocks(rom_bytes, BYPASS_MAGIC).into_iter().next().ok_or(ExGfxError::NotFound)?;
        let size = rats_size(rom_bytes, tag);
        // Payload follows the 8-byte magic (same layout as write_rats_block);
        // RATS size field = data_len - 1, hence the +1.
        Self::decode_payload(&rom_bytes[tag + 16..tag + 8 + size + 1])
    }

    /// Write the table to ROM: erase any existing block, allocate fresh free
    /// space, and write a new RATS-tagged block. Empty data erases the block
    /// without writing a new one.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), ExGfxError> {
        for tag in find_rats_blocks(rom_bytes, BYPASS_MAGIC) {
            let size = rats_size(rom_bytes, tag);
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }
        if self.levels.is_empty() {
            return Ok(());
        }
        let payload = self.encode_payload()?;
        write_rats_block(rom_bytes, BYPASS_MAGIC, &payload, header_offset)?;
        Ok(())
    }

    /// Slot value for `level`/`slot` (`None` = no bypass record for the level,
    /// i.e. every slot is [`BYPASS_DEFAULT`]).
    pub fn slot(&self, level: u16, slot: usize) -> Option<u16> {
        self.levels.get(&level).and_then(|s| s.get(slot).copied())
    }

    /// Set one slot; values of [`BYPASS_DEFAULT`] on all 8 slots erase the
    /// level's record. Returns an error for out-of-range levels/slots/values.
    pub fn set_slot(&mut self, level: u16, slot: usize, value: u16) -> Result<(), ExGfxError> {
        if level >= 0x200 {
            return Err(ExGfxError::BadLevel(level));
        }
        if slot >= BYPASS_SLOT_COUNT {
            return Err(ExGfxError::BadSlot(slot));
        }
        if value != BYPASS_DEFAULT && value > EXGFX_MAX_INDEX {
            return Err(ExGfxError::BadSlotValue(value));
        }
        let entry = self.levels.entry(level).or_insert([BYPASS_DEFAULT; BYPASS_SLOT_COUNT]);
        entry[slot] = value;
        if entry.iter().all(|&v| v == BYPASS_DEFAULT) {
            self.levels.remove(&level);
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// RATS helpers (shared by both block kinds)
// -------------------------------------------------------------------------------------------------

/// Scan `rom_bytes` for RATS blocks whose payload starts with `magic`.
/// Returns the file offsets of the `STAR` tags.
fn find_rats_blocks(rom_bytes: &[u8], magic: &[u8; 8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 16 < rom_bytes.len() {
        if &rom_bytes[i..i + 4] == b"STAR" {
            let size = u16::from_le_bytes([rom_bytes[i + 4], rom_bytes[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom_bytes[i + 6], rom_bytes[i + 7]]);
            if size as u16 ^ inv == 0xFFFF {
                let data_start = i + 8;
                // RATS size field = data length - 1, so the data runs to
                // data_start + size + 1.
                let data_end = data_start.saturating_add(size).saturating_add(1);
                if data_end <= rom_bytes.len()
                    && data_start + 8 <= rom_bytes.len()
                    && &rom_bytes[data_start..data_start + 8] == magic
                {
                    out.push(i);
                    i = data_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Payload size recorded in the RATS tag at `tag` (caller must have validated
/// the tag via [`find_rats_blocks`]).
fn rats_size(rom_bytes: &[u8], tag: usize) -> usize {
    u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize
}

/// Write one `STAR`-tagged block (`magic` + `payload`) into free space.
fn write_rats_block(
    rom_bytes: &mut [u8], magic: &[u8; 8], payload: &[u8], header_offset: usize,
) -> Result<(), ExGfxError> {
    if payload.is_empty() || payload.len() > 0x10000 - 8 {
        return Err(ExGfxError::TooLarge(payload.len()));
    }
    let total = 8 + 8 + payload.len(); // RATS tag + magic + payload
    let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
        .ok_or(ExGfxError::NoFreeSpace(total))?;
    let file_off = pc + header_offset;
    let size_field = (8 + payload.len() - 1) as u16;
    rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
    rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
    rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
    rom_bytes[file_off + 8..file_off + 16].copy_from_slice(magic);
    rom_bytes[file_off + 16..file_off + 16 + payload.len()].copy_from_slice(payload);
    Ok(())
}

// -------------------------------------------------------------------------------------------------
// Preview: apply the bypass to emulator VRAM
// -------------------------------------------------------------------------------------------------

/// VRAM byte offset + length for bypass `slot` (0-3 = FG/BG via
/// `OBJECT_SLOT_VRAM_RANGES`, 4-7 = sprites via `SPRITE_SLOT_VRAM_BASES`).
pub fn bypass_slot_vram_span(slot: usize) -> Option<(usize, usize)> {
    let base_tile: u16 =
        if slot < 4 { OBJECT_SLOT_VRAM_RANGES.get(slot)?.0 } else { SPRITE_SLOT_VRAM_BASES.get(slot - 4).copied()? };
    Some((base_tile as usize * EXGFX_TILE_BYTES, BYPASS_SLOT_BYTES))
}

/// Human-readable source label for a slot value, e.g. "GFX file 0C",
/// "ExGFX 80", or "Default (tileset)".
pub fn slot_source_label(value: u16) -> String {
    if value == BYPASS_DEFAULT {
        "Default (tileset)".to_string()
    } else if value < EXGFX_FIRST_INDEX {
        format!("GFX file {value:02X}")
    } else {
        format!("ExGFX {value:03X}")
    }
}

// NOTE: applying the bypass to emulator VRAM lives in
// `smwe_emu::emu::upload_gfx_file_to_vram` + the editor's bypass orchestration,
// not here. Vanilla files must go through the game's own `UploadGFXFile`
// routine for bit-exact 3bpp→4bpp expansion (a raw `memcpy` of the file's
// native bytes would render garbage), and that needs the emulator.

// -------------------------------------------------------------------------------------------------
// MWL section 7 payload
// -------------------------------------------------------------------------------------------------

/// Encode the MWL ExGFX/bypass section payload for `level`: whether the level
/// has a bypass record, plus every ExGFX file the record references.
///
/// ```text
/// u8:  bypass_present (0/1)
/// if present: level u16 LE, slots 8 x u16 LE
/// u16 LE: file count n
/// per file: index u16 LE, raw_len u16 LE, raw bytes
/// ```
pub fn encode_mwl_section(bypass: &BypassData, exgfx: &ExGfxData, level: u16) -> Vec<u8> {
    let mut out = Vec::new();
    match bypass.levels.get(&level) {
        Some(slots) => {
            out.push(1u8);
            out.extend_from_slice(&level.to_le_bytes());
            for &v in slots {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        None => out.push(0u8),
    }
    let mut files: Vec<&ExGfxFile> = Vec::new();
    if let Some(slots) = bypass.levels.get(&level) {
        for &v in slots {
            if v >= EXGFX_FIRST_INDEX {
                if let Some(f) = exgfx.files.get(&v) {
                    if !files.iter().any(|f2| f2.index == f.index) {
                        files.push(f);
                    }
                }
            }
        }
    }
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    for f in files {
        out.extend_from_slice(&f.index.to_le_bytes());
        out.extend_from_slice(&(f.raw.len() as u16).to_le_bytes());
        out.extend_from_slice(&f.raw);
    }
    out
}

/// Decode an MWL ExGFX/bypass section payload: the optional bypass record
/// `(level, slots)` plus the `(index, raw bytes)` file list.
pub fn decode_mwl_section(
    payload: &[u8],
) -> Result<(Option<(u16, [u16; BYPASS_SLOT_COUNT])>, Vec<(u16, Vec<u8>)>), ExGfxError> {
    let err = |m: &str| ExGfxError::Corrupt(m.to_string());
    if payload.is_empty() {
        return Err(err("empty ExGFX/bypass section"));
    }
    let mut off: usize;
    let bypass = match payload[0] {
        0 => {
            off = 1;
            None
        }
        1 => {
            if payload.len() < 1 + 2 + BYPASS_SLOT_COUNT * 2 {
                return Err(err("truncated bypass record"));
            }
            let level = u16::from_le_bytes([payload[1], payload[2]]);
            if level >= 0x200 {
                return Err(ExGfxError::BadLevel(level));
            }
            let mut slots = [BYPASS_DEFAULT; BYPASS_SLOT_COUNT];
            off = 3;
            for s in slots.iter_mut() {
                *s = u16::from_le_bytes([payload[off], payload[off + 1]]);
                off += 2;
            }
            Some((level, slots))
        }
        b => return Err(err(&format!("bad bypass_present flag {b}"))),
    };
    if payload.len() < off + 2 {
        return Err(err("truncated ExGFX file count"));
    }
    let n = u16::from_le_bytes([payload[off], payload[off + 1]]) as usize;
    off += 2;
    let mut files = Vec::with_capacity(n);
    for _ in 0..n {
        if payload.len() < off + 4 {
            return Err(err("truncated ExGFX file header"));
        }
        let index = u16::from_le_bytes([payload[off], payload[off + 1]]);
        let raw_len = u16::from_le_bytes([payload[off + 2], payload[off + 3]]) as usize;
        off += 4;
        if !(EXGFX_FIRST_INDEX..=EXGFX_MAX_INDEX).contains(&index) {
            return Err(ExGfxError::BadIndex(index));
        }
        if raw_len != EXGFX_FILE_BYTES {
            return Err(ExGfxError::BadSize { expected: EXGFX_FILE_BYTES, got: raw_len });
        }
        if payload.len() < off + raw_len {
            return Err(err("truncated ExGFX file data"));
        }
        files.push((index, payload[off..off + raw_len].to_vec()));
        off += raw_len;
    }
    Ok((bypass, files))
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank ROM image big enough for free-space allocation tests
    /// (1 MiB of 0xFF, no SMC header).
    fn blank_rom() -> Vec<u8> {
        vec![0xFF; 0x100000]
    }

    /// Deterministic pseudo-random 4bpp tile bytes (so tests can tell files
    /// apart without a real ROM).
    fn patterned_raw(seed: u8) -> Vec<u8> {
        (0..EXGFX_FILE_BYTES).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    #[test]
    fn insert_parse_round_trip() {
        let mut rom = blank_rom();
        let mut data = ExGfxData::default();
        data.insert_raw(0x80, patterned_raw(1)).unwrap();
        data.insert_raw(0x81, patterned_raw(2)).unwrap();
        data.write_to_rom(&mut rom, 0).unwrap();

        let parsed = ExGfxData::parse(&rom);
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[&0x80].raw_bytes(), patterned_raw(1).as_slice());
        assert_eq!(parsed.files[&0x81].raw_bytes(), patterned_raw(2).as_slice());
        assert_eq!(parsed.files[&0x80].tiles.len(), EXGFX_TILES_PER_FILE);
    }

    #[test]
    fn remove_erases_block() {
        let mut rom = blank_rom();
        let mut data = ExGfxData::default();
        data.insert_raw(0x80, patterned_raw(1)).unwrap();
        data.write_to_rom(&mut rom, 0).unwrap();
        assert_eq!(ExGfxData::parse(&rom).files.len(), 1);

        data.remove(0x80);
        data.write_to_rom(&mut rom, 0).unwrap();
        assert!(ExGfxData::parse(&rom).files.is_empty());
        // The erased block reads as free space again.
        assert!(find_rats_blocks(&rom, EXGFX_MAGIC).is_empty());
    }

    #[test]
    fn insert_rejects_bad_sizes_and_indices() {
        let mut data = ExGfxData::default();
        assert!(matches!(data.insert_raw(0x80, vec![0u8; 100]), Err(ExGfxError::BadSize { .. })));
        assert!(matches!(data.insert_raw(0x7F, patterned_raw(0)), Err(ExGfxError::BadIndex(_))));
    }

    #[test]
    fn bypass_round_trip() {
        let mut rom = blank_rom();
        let mut bypass = BypassData::default();
        // No block on a fresh ROM.
        assert!(matches!(BypassData::parse(&rom), Err(ExGfxError::NotFound)));
        bypass.set_slot(0x105, 0, 0x80).unwrap();
        bypass.set_slot(0x105, 4, 0x0C).unwrap();
        bypass.write_to_rom(&mut rom, 0).unwrap();

        let parsed = BypassData::parse(&rom).unwrap();
        assert_eq!(parsed.slot(0x105, 0), Some(0x80));
        assert_eq!(parsed.slot(0x105, 4), Some(0x0C));
        assert_eq!(parsed.slot(0x105, 1), Some(BYPASS_DEFAULT));
        assert_eq!(parsed.slot(0x106, 0), None);

        // Resetting every slot to default erases the level's record.
        let mut b2 = parsed;
        for s in 0..BYPASS_SLOT_COUNT {
            b2.set_slot(0x105, s, BYPASS_DEFAULT).unwrap();
        }
        b2.write_to_rom(&mut rom, 0).unwrap();
        assert!(matches!(BypassData::parse(&rom), Err(ExGfxError::NotFound)));
    }

    #[test]
    fn bypass_rejects_bad_values() {
        let mut bypass = BypassData::default();
        assert!(matches!(bypass.set_slot(0x200, 0, 0x80), Err(ExGfxError::BadLevel(_))));
        assert!(matches!(bypass.set_slot(0x105, 8, 0x80), Err(ExGfxError::BadSlot(_))));
    }

    #[test]
    fn slot_span_layout() {
        // FG slots: 0x80 tiles each starting at tile 0x000.
        assert_eq!(bypass_slot_vram_span(0), Some((0x0000, 0x1000)));
        assert_eq!(bypass_slot_vram_span(3), Some((0x180 * 32, 0x1000)));
        // Sprite slots via SPRITE_SLOT_VRAM_BASES.
        assert_eq!(bypass_slot_vram_span(4), Some((0x780 * 32, 0x1000)));
        assert_eq!(bypass_slot_vram_span(7), Some((0x600 * 32, 0x1000)));
        assert_eq!(bypass_slot_vram_span(8), None);
    }

    #[test]
    fn mwl_section_round_trip() {
        let mut exgfx = ExGfxData::default();
        exgfx.insert_raw(0x80, patterned_raw(7)).unwrap();
        let mut bypass = BypassData::default();
        bypass.set_slot(0x105, 0, 0x80).unwrap();
        bypass.set_slot(0x105, 5, 0x0D).unwrap();

        let payload = encode_mwl_section(&bypass, &exgfx, 0x105);
        let (b, files) = decode_mwl_section(&payload).unwrap();
        let (level, slots) = b.unwrap();
        assert_eq!(level, 0x105);
        assert_eq!(slots[0], 0x80);
        assert_eq!(slots[5], 0x0D);
        assert_eq!(slots[1], BYPASS_DEFAULT);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, 0x80);
        assert_eq!(files[0].1, patterned_raw(7));

        // A level with no bypass record exports an empty section body.
        let payload2 = encode_mwl_section(&bypass, &exgfx, 0x106);
        let (b2, files2) = decode_mwl_section(&payload2).unwrap();
        assert!(b2.is_none());
        assert!(files2.is_empty());
    }

    #[test]
    fn mwl_section_rejects_truncation() {
        assert!(decode_mwl_section(&[]).is_err());
        assert!(decode_mwl_section(&[1, 0]).is_err());
        let mut exgfx = ExGfxData::default();
        exgfx.insert_raw(0x80, patterned_raw(7)).unwrap();
        let mut bypass = BypassData::default();
        bypass.set_slot(0x105, 0, 0x80).unwrap();
        let mut payload = encode_mwl_section(&bypass, &exgfx, 0x105);
        assert!(payload.len() > 10);
        payload.truncate(payload.len() - 10);
        assert!(decode_mwl_section(&payload).is_err());
    }

    #[test]
    fn parse_ignores_unrelated_rats_blocks() {
        let mut rom = blank_rom();
        // Some other tool's RATS block (e.g. the ExAnimation one).
        rom[0x100..0x104].copy_from_slice(b"STAR");
        rom[0x104..0x106].copy_from_slice(&7u16.to_le_bytes());
        rom[0x106..0x108].copy_from_slice(&(!7u16).to_le_bytes());
        rom[0x108..0x110].copy_from_slice(b"SMWEXAN1");
        let mut data = ExGfxData::default();
        data.insert_raw(0x80, patterned_raw(3)).unwrap();
        data.write_to_rom(&mut rom, 0).unwrap();
        let parsed = ExGfxData::parse(&rom);
        assert_eq!(parsed.files.len(), 1);
        assert!(matches!(BypassData::parse(&rom), Err(ExGfxError::NotFound)));
    }

    /// Real-ROM ignored test: expand a scratch copy of the real ROM to 4 MB
    /// (a vanilla 512 KiB ROM has ~37 KiB free total — like in LM, serious
    /// ExGFX work wants an expanded ROM), insert an ExGFX file + bypass
    /// record, re-parse, and verify the data round-trips. Also verifies
    /// `smwe_emu::emu::upload_gfx_file_to_vram` is bit-exact against the
    /// game's own level-load upload (the Super GFX Bypass preview relies on
    /// it for vanilla-file slots).
    #[test]
    #[ignore]
    fn real_rom_insert_bypass_and_vram_apply() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM");
        let rom_bytes = std::fs::read(&path).expect("read ROM");
        let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };
        let (smc_header, body) = rom_bytes.split_at(header_offset);
        let expanded = crate::rom_expansion::expand_rom(&crate::Rom::new(body.to_vec()).unwrap(), 0x40_0000)
            .expect("expand scratch ROM");
        let mut rom_bytes = smc_header.to_vec();
        rom_bytes.extend_from_slice(expanded.bytes());

        // Bit-exactness: the game's UploadGFXFile via upload_gfx_file_to_vram
        // must reproduce decompress_sublevel's FG1 bytes exactly. (A raw
        // memcpy of a 3bpp file would NOT — the routine expands 3bpp→4bpp
        // with Nintendo's tile-dependent bitplane-3 mask.)
        {
            let mut emu_rom = smwe_emu::rom::Rom::new(rom_bytes[header_offset..].to_vec());
            emu_rom.load_symbols(include_str!("../../../symbols/SMW_U.sym"));
            let mut cpu = smwe_emu::Cpu::new(smwe_emu::emu::CheckedMem::new(std::sync::Arc::new(emu_rom)));
            smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
            let pristine = crate::SmwRom::from_rom(crate::Rom::new(rom_bytes.clone()).unwrap()).unwrap();
            let tileset = pristine.levels[0x105].primary_header.fg_bg_gfx() as usize;
            let fg_files = pristine.gfx.object_gfx_list.files_for_object_tileset(tileset);
            let (off, len) = bypass_slot_vram_span(0).unwrap();
            let before = cpu.mem.vram[off..off + len].to_vec();
            smwe_emu::emu::upload_gfx_file_to_vram(&mut cpu, fg_files[0] as u8, 0x0000);
            assert_eq!(
                &cpu.mem.vram[off..off + len],
                &before[..],
                "upload_gfx_file_to_vram must be bit-exact vs the game's level upload"
            );
        }

        let mut exgfx = ExGfxData::parse(&rom_bytes);
        exgfx.insert_raw(0x80, patterned_raw(0x42)).unwrap();
        exgfx.write_to_rom(&mut rom_bytes, header_offset).unwrap();

        let mut bypass = BypassData::parse(&rom_bytes).unwrap_or_default();
        bypass.set_slot(0x105, 0, 0x80).unwrap(); // FG1 -> ExGFX 0x80
        bypass.write_to_rom(&mut rom_bytes, header_offset).unwrap();

        // Re-parse from the modified bytes, like a fresh project load.
        let smw = crate::SmwRom::from_rom(crate::Rom::new(rom_bytes).unwrap()).unwrap();
        assert_eq!(smw.exgfx.files[&0x80].raw_bytes(), patterned_raw(0x42).as_slice());
        assert_eq!(smw.gfx_bypass.slot(0x105, 0), Some(0x80));

        // ExGFX VRAM contract (the editor's bypass orchestration does this
        // memcpy): the file's first BYPASS_SLOT_BYTES land on the slot span.
        let (off, len) = bypass_slot_vram_span(0).unwrap();
        assert_eq!(len, BYPASS_SLOT_BYTES);
        assert_eq!(&smw.exgfx.files[&0x80].raw_bytes()[..len], &patterned_raw(0x42)[..len]);
        let _ = off;
    }
}
