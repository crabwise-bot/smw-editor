//! Vanilla and custom (LM v2.50+) overworld sprites.
//!
//! ## Vanilla sprites
//!
//! The 13 fixed sprite records live at SNES `$04F625`, 5 bytes each:
//! `sprite_number`, little-endian X, little-endian Y. Positions are pixels in
//! overworld map space (the main map is 512×512 px).
//!
//! ## Submap visibility ("inactivity" bytes)
//!
//! Whether a sprite runs on the current submap is decided per frame by
//! `CODE_04F87C` (`bank_04.asm` in SMWDisX). Verified against the real ROM
//! (2026-09-17), the relevant instructions are:
//!
//! ```text
//!   LDY.W OWSpriteNumber,X   ; Y = sprite NUMBER (dispatch ID 0x00-0x0A)...
//!   LDA.W $F828,Y            ; ...NOT the table slot!
//!   ...
//!   LDA.W $F875,Y            ; submap bit mask: $80,$40,$20,$10,$08,$04,$02
//!   AND $00
//!   BEQ draw                 ; (byte & mask) == 0 -> sprite is active
//! ```
//!
//! Consequences, all confirmed byte-for-byte against the vanilla ROM:
//!
//! * The visibility table is indexed by **sprite number**, not by table slot.
//!   Slots that share a sprite number (e.g. the three ghosts, all number
//!   `0x0A`) share one visibility byte.
//! * A set bit means **inactive** on that submap. Bit order, confirmed by
//!   `DATA_04F875` (`db $80,$40,$20,$10,$08,$04,$02`): bit 7 = main map,
//!   bit 6 = Yoshi's Island, bit 5 = Vanilla Dome, bit 4 = Forest of Illusion,
//!   bit 3 = Valley of Bowser, bit 2 = Special World, bit 1 = Star World,
//!   bit 0 = unused.
//! * Sprite number 0's "visibility byte" is the byte at `$04F828` — the `RTS`
//!   opcode (`$60`) terminating the sprite engine. It must never be edited.
//!   (With `$60`, number-0 sprites are active on the main map, Forest, Valley,
//!   Special and Star World, and inactive on Yoshi's Island/Vanilla Dome.)
//!
//! Sanity checks against vanilla gameplay: the Valley of Bowser sign (number
//! `0x08`) reads `$F7` → active only on the Valley of Bowser submap; Yoshi's
//! house smoke (number `0x07`) reads `$3F` → active on the main map; the
//! ghosts (number `0x0A`) read `$00` → active everywhere.
//!
//! ## Custom sprites (LM v2.50+)
//!
//! Lunar Magic v2.50 added a separate custom-sprite table reached through the
//! 3-byte SNES pointer at `$0EF55D` (`FF FF FF` = no table). The table starts
//! with seven little-endian 16-bit offsets, one per submap, and holds up to 24
//! custom sprites per submap. Each entry is
//! `xnnnnnnn yyyXXXXX hhhhhYYY eeeeeeee…`: 7-bit sprite number, 6-bit X and Y
//! in 8×8 units (bit 5 of X rides in bit 7 of the first byte), 5-bit height,
//! then a variable number of extra bytes (default 1; LM v3.51 may provide a
//! 0x80-entry per-sprite count table whose 3-byte pointer lives at `$0DE18C`
//! with marker byte `$42` at `$0DE18F`).
//!
//! ## Custom sprite list sizes (LM v3.51)
//!
//! Lunar Magic 3.51 (2024-12-25) added support for *custom overworld sprite
//! list sizes*: each submap's custom sprite list has a configurable capacity
//! (how many custom sprites that submap may hold), instead of one fixed cap
//! for every submap. The documented native maximum is 24 per submap
//! ([Overworld Data Format](https://smwspeedruns.com/Overworld_Data_Format),
//! accurate as of LM 3.51).
//!
//! smw-editor models this as `CustomSpriteTable::list_sizes` — one capacity
//! per submap, default 24, hard-capped at 24 — persisted in the `OWSPRITE`
//! RATS payload (format version 2). Version-1 payloads (written before this
//! feature) decode with all sizes defaulted to 24, so old saves load
//! unchanged.
//!
//! **Important:** Lunar Magic only *authors* this table. Custom sprites do
//! nothing in-game unless a third-party runtime patch (e.g. a custom-sprite
//! engine) is also installed — vanilla SMW has no code that reads the table.
//! smw-editor stores the table in the same spirit: data, not behavior.
//!
//! LM's exact on-disk layout for the table is not publicly documented, so
//! smw-editor uses its own clearly-marked RATS-tagged format (magic
//! `OWSPRITE`, version 2): the 3-byte pointer at `$0EF55D` aims at the `STAR`
//! tag, and the seven submap offsets are byte offsets from the start of the
//! RATS payload (`0xFFFF` = submap has no custom sprites).

use crate::snes_utils::{
    addr::{AddrPc, AddrSnes},
    rom::Rom,
};

/// SNES address of the 13 vanilla overworld sprite records (5 bytes each).
pub const VANILLA_SPRITE_TABLE_SNES: AddrSnes = AddrSnes(0x04F625);
/// Number of fixed vanilla sprite records.
pub const VANILLA_SPRITE_COUNT: usize = 13;
/// Bytes per vanilla sprite record: number, x LE, y LE.
pub const VANILLA_SPRITE_RECORD_LEN: usize = 5;

/// SNES address of the byte read as sprite number 0's visibility byte.
/// This is the `RTS` opcode terminating the sprite engine — read-only.
pub const NUMBER0_VISIBILITY_SNES: AddrSnes = AddrSnes(0x04F828);
/// SNES address of the 10 visibility bytes for sprite numbers 1..=10.
pub const VISIBILITY_TABLE_SNES: AddrSnes = AddrSnes(0x04F829);
/// Visibility bytes, indexed by sprite number 0..=10. Entry 0 is the engine's
/// `RTS` opcode (`$60`) and must never be written.
pub const VISIBILITY_COUNT: usize = 11;

/// 3-byte SNES pointer (little-endian) to the custom overworld sprite table.
/// `FF FF FF` = no custom sprite table.
pub const CUSTOM_SPRITE_PTR_SNES: AddrSnes = AddrSnes(0x0EF55D);
/// 3-byte SNES pointer to the 0x80-entry extra-byte-count table (LM v3.51+).
pub const EXTRA_BYTE_COUNT_PTR_SNES: AddrSnes = AddrSnes(0x0DE18C);
/// Marker byte address: when this holds [`EXTRA_BYTE_COUNT_MARKER`], the
/// extra-byte-count table is present.
pub const EXTRA_BYTE_COUNT_MARKER_SNES: AddrSnes = AddrSnes(0x0DE18F);
pub const EXTRA_BYTE_COUNT_MARKER: u8 = 0x42;
/// Default extra bytes per custom sprite when no count table is present.
pub const DEFAULT_EXTRA_BYTES: usize = 1;
/// Maximum custom sprites per submap: the documented native LM limit
/// (smwspeedruns "Overworld Data Format", accurate as of LM 3.51). Per-submap
/// list sizes ([`CustomSpriteTable::list_sizes`]) may be set lower, never
/// higher.
pub const MAX_CUSTOM_SPRITES_PER_SUBMAP: usize = 24;
/// Default per-submap custom sprite list size (LM v3.51 "custom overworld
/// sprite list sizes" — each submap's list capacity, configurable 0..=24).
pub const DEFAULT_CUSTOM_LIST_SIZE: u8 = MAX_CUSTOM_SPRITES_PER_SUBMAP as u8;

/// Visibility bit per submap index 0..=6: bit set = sprite is INACTIVE there.
/// Matches `DATA_04F875` (`db $80,$40,$20,$10,$08,$04,$02`) in `bank_04.asm`.
pub const SUBMAP_VISIBILITY_BITS: [u8; 7] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02];

/// Highest valid vanilla sprite dispatch ID.
pub const MAX_VANILLA_SPRITE_NUMBER: u8 = 0x0A;
/// Highest valid custom sprite number (7-bit field).
pub const MAX_CUSTOM_SPRITE_NUMBER: u8 = 0x7F;

/// Display names for the vanilla sprite dispatch IDs `0x00..=0x0A`.
///
/// Confirmed against SMWDisX `bank_04.asm` (2026-09-17): `0x03` = overworld
/// fish, `0x04` = Piranha Plant, `0x05` = moving cloud, `0x06` = Koopa Kid,
/// `0x07` = Yoshi's House smoke, `0x08` = Bowser sign, `0x0A` = overworld
/// ghost. `0x01`/`0x02` are the believed-unused Lakitu/Blue Jay (neither is
/// placed in the vanilla table) and `0x09`'s identity is unverified, so those
/// stay honestly labeled. `0x00`'s handler is a bare `RTS` — its slots draw
/// nothing and behave as invisible position markers.
pub const SPRITE_TYPE_NAMES: [&str; 11] = [
    "Unknown (no draw routine)",
    "Lakitu (unused?)",
    "Blue Jay (unused?)",
    "Jumping fish",
    "Piranha Plant",
    "Moving cloud",
    "Koopa Kid",
    "Yoshi's House smoke",
    "Valley of Bowser sign",
    "Unknown",
    "Ghost",
];

/// Display name for a vanilla sprite dispatch ID.
pub fn sprite_type_name(number: u8) -> &'static str {
    SPRITE_TYPE_NAMES.get(number as usize).copied().unwrap_or("Invalid")
}

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SpriteError {
    #[error("address conversion failed: {0}")]
    Address(#[from] crate::snes_utils::addr::AddressError),
    #[error("vanilla sprite table overruns ROM")]
    TableOverrunsRom,
    #[error("custom sprite pointer overruns ROM")]
    PointerOverrunsRom,
    #[error("custom sprite table overruns ROM")]
    CustomTableOverrunsRom,
    #[error("custom sprite data is corrupt: {0}")]
    Corrupt(String),
    #[error("no free space for {0} custom-sprite bytes")]
    NoFreeSpace(usize),
    #[error("custom sprite table payload too large ({0} bytes)")]
    TooLarge(usize),
    #[error("sprite number {0:#04X} has no editable visibility byte")]
    NoVisibilityByte(u8),
    #[error("refusing to overwrite a custom sprite table not authored by smw-editor")]
    ForeignTable,
    #[error("cannot shrink submap {submap}'s custom sprite list to {size}: it holds {count} sprites")]
    ListSizeTooSmall { submap: usize, size: u8, count: usize },
}

// -------------------------------------------------------------------------------------------------
// Vanilla sprites
// -------------------------------------------------------------------------------------------------

/// One fixed vanilla overworld sprite record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VanillaOwSprite {
    /// Dispatch ID `0x00..=0x0A` (jump table at `CODE_04F853`).
    pub number: u8,
    /// X position in pixels (little-endian in ROM; signed for display).
    pub x:      u16,
    /// Y position in pixels (little-endian in ROM; signed for display).
    pub y:      u16,
}

impl VanillaOwSprite {
    pub fn x_px(&self) -> i16 {
        self.x as i16
    }

    pub fn y_px(&self) -> i16 {
        self.y as i16
    }

    pub fn type_name(&self) -> &'static str {
        sprite_type_name(self.number)
    }
}

/// The vanilla overworld sprite table plus per-number visibility bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VanillaOwSprites {
    pub sprites:    [VanillaOwSprite; VANILLA_SPRITE_COUNT],
    /// Visibility bytes indexed by sprite number `0..=10`. Bit set =
    /// inactive on that submap (see [`SUBMAP_VISIBILITY_BITS`]). Entry 0 is
    /// the sprite engine's `RTS` opcode and is never written back.
    pub visibility: [u8; VISIBILITY_COUNT],
}

impl VanillaOwSprites {
    /// Parse the vanilla table from raw ROM bytes (SMC header aware via
    /// `header_offset`: 0x200 with header, 0 without).
    pub fn parse(rom_bytes: &[u8], header_offset: usize) -> Result<Self, SpriteError> {
        let table_pc = AddrPc::try_from_lorom(VANILLA_SPRITE_TABLE_SNES)?.as_index() + header_offset;
        let table_end = table_pc + VANILLA_SPRITE_COUNT * VANILLA_SPRITE_RECORD_LEN;
        let vis_pc = AddrPc::try_from_lorom(NUMBER0_VISIBILITY_SNES)?.as_index() + header_offset;
        let vis_end = vis_pc + VISIBILITY_COUNT;
        if table_end > rom_bytes.len() || vis_end > rom_bytes.len() {
            return Err(SpriteError::TableOverrunsRom);
        }
        let mut sprites = [VanillaOwSprite { number: 0, x: 0, y: 0 }; VANILLA_SPRITE_COUNT];
        for (i, slot) in sprites.iter_mut().enumerate() {
            let off = table_pc + i * VANILLA_SPRITE_RECORD_LEN;
            *slot = VanillaOwSprite {
                number: rom_bytes[off],
                x:      u16::from_le_bytes([rom_bytes[off + 1], rom_bytes[off + 2]]),
                y:      u16::from_le_bytes([rom_bytes[off + 3], rom_bytes[off + 4]]),
            };
        }
        let mut visibility = [0u8; VISIBILITY_COUNT];
        visibility.copy_from_slice(&rom_bytes[vis_pc..vis_end]);
        Ok(Self { sprites, visibility })
    }

    /// Write the table back. The number-0 byte (`$04F828`, the engine `RTS`)
    /// is deliberately never written.
    pub fn write(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), SpriteError> {
        let table_pc = AddrPc::try_from_lorom(VANILLA_SPRITE_TABLE_SNES)?.as_index() + header_offset;
        let table_end = table_pc + VANILLA_SPRITE_COUNT * VANILLA_SPRITE_RECORD_LEN;
        let vis_pc = AddrPc::try_from_lorom(VISIBILITY_TABLE_SNES)?.as_index() + header_offset;
        let vis_end = vis_pc + (VISIBILITY_COUNT - 1);
        if table_end > rom_bytes.len() || vis_end > rom_bytes.len() {
            return Err(SpriteError::TableOverrunsRom);
        }
        for (i, sprite) in self.sprites.iter().enumerate() {
            let off = table_pc + i * VANILLA_SPRITE_RECORD_LEN;
            rom_bytes[off] = sprite.number;
            rom_bytes[off + 1..off + 3].copy_from_slice(&sprite.x.to_le_bytes());
            rom_bytes[off + 3..off + 5].copy_from_slice(&sprite.y.to_le_bytes());
        }
        rom_bytes[vis_pc..vis_end].copy_from_slice(&self.visibility[1..]);
        Ok(())
    }

    /// Whether sprite `number` is active on `submap` (0..=6), per
    /// `CODE_04F87C`: active iff `(visibility[number] & mask) == 0`.
    pub fn is_active_on(&self, number: u8, submap: u8) -> bool {
        let (Some(&byte), Some(&mask)) =
            (self.visibility.get(number as usize), SUBMAP_VISIBILITY_BITS.get(submap as usize))
        else {
            return false;
        };
        byte & mask == 0
    }

    /// Set whether sprite `number` (1..=10) is active on `submap`.
    /// Number 0 has no editable byte (it would clobber the engine `RTS`).
    pub fn set_active_on(&mut self, number: u8, submap: u8, active: bool) -> Result<(), SpriteError> {
        let byte = self.visibility.get_mut(number as usize).ok_or(SpriteError::NoVisibilityByte(number))?;
        if number == 0 {
            return Err(SpriteError::NoVisibilityByte(number));
        }
        let mask = SUBMAP_VISIBILITY_BITS.get(submap as usize).copied().unwrap_or(0);
        if active {
            *byte &= !mask;
        } else {
            *byte |= mask;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// Custom sprites (LM v2.50+)
// -------------------------------------------------------------------------------------------------

/// One custom overworld sprite entry.
///
/// Positions are 6-bit values in 8×8-pixel units (`pixels = units * 8`);
/// height is a 5-bit value; extra bytes are opaque to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomOwSprite {
    /// Sprite number `0x00..=0x7F`.
    pub number: u8,
    /// X in 8px units (6-bit).
    pub x:      u8,
    /// Y in 8px units (6-bit).
    pub y:      u8,
    /// Height (5-bit).
    pub height: u8,
    /// Extra bytes (default 1; see [`extra_byte_counts`]).
    pub extra:  Vec<u8>,
}

impl CustomOwSprite {
    pub fn x_px(&self) -> u16 {
        self.x as u16 * 8
    }

    pub fn y_px(&self) -> u16 {
        self.y as u16 * 8
    }

    /// Decode one entry: 3 fixed bytes + `extra_len` extra bytes.
    fn decode(data: &[u8], extra_len: usize) -> Option<Self> {
        let bytes = data.get(..3 + extra_len)?;
        let b0 = bytes[0];
        let b1 = bytes[1];
        let b2 = bytes[2];
        Some(Self {
            number: b0 & 0x7F,
            x:      ((b0 >> 7) << 5) | (b1 & 0x1F),
            y:      ((b1 >> 5) << 3) | (b2 & 0x07),
            height: (b2 >> 3) & 0x1F,
            extra:  bytes[3..3 + extra_len].to_vec(),
        })
    }

    /// Encode one entry: 3 fixed bytes + exactly `extra_len` extra bytes
    /// (padded with zeros or truncated).
    fn encode(&self, extra_len: usize) -> Vec<u8> {
        let b0 = (self.number & 0x7F) | ((self.x >> 5) << 7);
        let b1 = (((self.y >> 3) & 0x07) << 5) | (self.x & 0x1F);
        let b2 = ((self.height & 0x1F) << 3) | (self.y & 0x07);
        let mut out = vec![b0, b1, b2];
        out.extend(self.extra.iter().copied().take(extra_len));
        out.resize(3 + extra_len, 0);
        out
    }
}

/// Custom overworld sprites, one list per submap (index 0..=6).
///
/// `list_sizes` is the LM v3.51 "custom overworld sprite list sizes" feature:
/// the configured capacity (maximum custom sprite count) of each submap's
/// list, 0..=[`MAX_CUSTOM_SPRITES_PER_SUBMAP`]. The editor refuses to insert
/// past a submap's configured size, mirroring LM's "not enough room" save
/// rejection. Defaults to 24 per submap (the documented native maximum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomSpriteTable {
    pub submaps:    [Vec<CustomOwSprite>; 7],
    pub list_sizes: [u8; 7],
}

impl Default for CustomSpriteTable {
    fn default() -> Self {
        Self { submaps: Default::default(), list_sizes: [DEFAULT_CUSTOM_LIST_SIZE; 7] }
    }
}

impl CustomSpriteTable {
    pub fn is_empty(&self) -> bool {
        self.submaps.iter().all(Vec::is_empty)
    }

    pub fn total_count(&self) -> usize {
        self.submaps.iter().map(Vec::len).sum()
    }

    /// Configured list size (capacity) for `submap` (0..=6), clamped to the
    /// valid range. Out-of-range submaps read as 0.
    pub fn list_size(&self, submap: usize) -> u8 {
        self.list_sizes.get(submap).copied().unwrap_or(0).min(MAX_CUSTOM_SPRITES_PER_SUBMAP as u8)
    }

    /// Free slots left on `submap`'s custom sprite list.
    pub fn room_for(&self, submap: usize) -> usize {
        (self.list_size(submap) as usize).saturating_sub(self.submaps.get(submap).map(Vec::len).unwrap_or(0))
    }

    /// Set submap `submap`'s list size. Sizes above
    /// [`MAX_CUSTOM_SPRITES_PER_SUBMAP`] are clamped to it; shrinking below
    /// the number of sprites the list currently holds is refused with
    /// [`SpriteError::ListSizeTooSmall`].
    pub fn set_list_size(&mut self, submap: usize, size: u8) -> Result<(), SpriteError> {
        let size = size.min(MAX_CUSTOM_SPRITES_PER_SUBMAP as u8);
        let count = self.submaps.get(submap).map(Vec::len).unwrap_or(0);
        if (size as usize) < count {
            return Err(SpriteError::ListSizeTooSmall { submap, size, count });
        }
        if let Some(slot) = self.list_sizes.get_mut(submap) {
            *slot = size;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// Custom table ROM storage (smw-editor RATS format)
// -------------------------------------------------------------------------------------------------

/// Magic at the start of our custom-sprite RATS payload.
const CUSTOM_MAGIC: &[u8; 8] = b"OWSPRITE";
/// Version 2 adds the 7 per-submap list sizes (LM v3.51 parity) after the
/// submap offsets. Version-1 payloads (written before this feature) decode
/// with every list size defaulted to [`DEFAULT_CUSTOM_LIST_SIZE`].
const CUSTOM_VERSION: u8 = 2;
/// Payload header: magic + version + 7 submap offsets + 7 list sizes.
const CUSTOM_HEADER_LEN: usize = 8 + 1 + 7 * 2 + 7;
/// Header length of version-1 payloads (no list sizes).
const CUSTOM_HEADER_LEN_V1: usize = 8 + 1 + 7 * 2;
/// Submap offset value meaning "this submap has no custom sprites".
const NO_SPRITES_OFFSET: u16 = 0xFFFF;

/// Read a 3-byte little-endian SNES address, or `None` for `FF FF FF`.
fn read_snes3(rom_bytes: &[u8], pc: usize) -> Option<AddrSnes> {
    let b = rom_bytes.get(pc..pc + 3)?;
    if b == [0xFF, 0xFF, 0xFF] {
        return None;
    }
    Some(AddrSnes(u32::from_le_bytes([b[0], b[1], b[2], 0])))
}

/// Per-sprite extra-byte counts: the 0x80-entry table when LM v3.51's marker
/// is present, otherwise [`DEFAULT_EXTRA_BYTES`] for every sprite number.
pub fn extra_byte_counts(rom_bytes: &[u8], header_offset: usize) -> [u8; 128] {
    let mut counts = [DEFAULT_EXTRA_BYTES as u8; 128];
    let (Ok(ptr_pc), Ok(marker_pc)) = (
        AddrPc::try_from_lorom(EXTRA_BYTE_COUNT_PTR_SNES).map(|p| p.as_index() + header_offset),
        AddrPc::try_from_lorom(EXTRA_BYTE_COUNT_MARKER_SNES).map(|p| p.as_index() + header_offset),
    ) else {
        return counts;
    };
    if rom_bytes.get(marker_pc).copied() != Some(EXTRA_BYTE_COUNT_MARKER) {
        return counts;
    }
    let Some(table_snes) = read_snes3(rom_bytes, ptr_pc) else { return counts };
    let Ok(table_pc) = AddrPc::try_from_lorom(table_snes).map(|p| p.as_index() + header_offset) else {
        return counts;
    };
    if let Some(table) = rom_bytes.get(table_pc..table_pc + 128) {
        counts.copy_from_slice(table);
    }
    counts
}

/// Validate a RATS tag at `file_off`; returns the payload range on success.
fn rats_payload_range(rom_bytes: &[u8], file_off: usize) -> Option<std::ops::Range<usize>> {
    let tag = rom_bytes.get(file_off..file_off + 8)?;
    if &tag[..4] != b"STAR" {
        return None;
    }
    let size = u16::from_le_bytes([tag[4], tag[5]]) as usize;
    let inv = u16::from_le_bytes([tag[6], tag[7]]);
    if size as u16 ^ inv != 0xFFFF {
        return None;
    }
    let start = file_off + 8;
    let end = start + size + 1;
    if end > rom_bytes.len() {
        return None;
    }
    Some(start..end)
}

/// Erase the RATS block at `file_off` (fill with `0xFF` = free space).
fn erase_rats_block(rom_bytes: &mut [u8], file_off: usize) {
    if let Some(range) = rats_payload_range(rom_bytes, file_off) {
        rom_bytes[file_off..range.end].fill(0xFF);
    }
}

impl CustomSpriteTable {
    /// Encode to the RATS payload format (without the `STAR` tag), version 2.
    /// Each submap's list is truncated to its configured list size.
    pub fn encode_payload(&self, extra_counts: &[u8; 128]) -> Vec<u8> {
        let mut lists: Vec<Vec<u8>> = Vec::with_capacity(7);
        for (submap, sprites) in self.submaps.iter().enumerate() {
            let cap = self.list_size(submap) as usize;
            let mut list = Vec::with_capacity(1 + sprites.len().min(cap) * 4);
            let stored = sprites.len().min(cap);
            list.push(stored as u8);
            for sprite in sprites.iter().take(cap) {
                let n = extra_counts[(sprite.number & 0x7F) as usize] as usize;
                list.extend_from_slice(&sprite.encode(n));
            }
            lists.push(list);
        }
        let mut out = Vec::with_capacity(CUSTOM_HEADER_LEN + lists.iter().map(Vec::len).sum::<usize>());
        out.extend_from_slice(CUSTOM_MAGIC);
        out.push(CUSTOM_VERSION);
        let mut offset = CUSTOM_HEADER_LEN;
        for list in &lists {
            if list.len() <= 1 {
                // Count byte only (zero sprites): no list stored.
                out.extend_from_slice(&NO_SPRITES_OFFSET.to_le_bytes());
            } else {
                out.extend_from_slice(&(offset as u16).to_le_bytes());
                offset += list.len();
            }
        }
        out.extend_from_slice(&self.list_sizes);
        for list in &lists {
            if list.len() > 1 {
                out.extend_from_slice(list);
            }
        }
        out
    }

    /// Decode a RATS payload produced by [`Self::encode_payload`]. Accepts
    /// version 1 (no list sizes — every size defaults to
    /// [`DEFAULT_CUSTOM_LIST_SIZE`]) and version 2.
    pub fn decode_payload(payload: &[u8], extra_counts: &[u8; 128]) -> Result<Self, SpriteError> {
        let corrupt = |msg: &str| SpriteError::Corrupt(msg.to_string());
        if payload.len() < CUSTOM_HEADER_LEN_V1 {
            return Err(corrupt("payload shorter than header"));
        }
        if &payload[..8] != CUSTOM_MAGIC {
            return Err(corrupt("bad magic"));
        }
        let version = payload[8];
        let sizes: [u8; 7] = match version {
            1 => [DEFAULT_CUSTOM_LIST_SIZE; 7],
            CUSTOM_VERSION => {
                if payload.len() < CUSTOM_HEADER_LEN {
                    return Err(corrupt("payload shorter than v2 header"));
                }
                let mut s = [0u8; 7];
                s.copy_from_slice(&payload[23..30]);
                if s.iter().any(|&n| n as usize > MAX_CUSTOM_SPRITES_PER_SUBMAP) {
                    return Err(corrupt("list size exceeds 24"));
                }
                s
            }
            v => return Err(SpriteError::Corrupt(format!("unsupported version {v}"))),
        };
        // Submap offsets live at bytes 9..23 in both versions.
        let mut table = CustomSpriteTable::default();
        table.list_sizes = sizes;
        for (submap, sprites) in table.submaps.iter_mut().enumerate() {
            let off = u16::from_le_bytes([payload[9 + submap * 2], payload[9 + submap * 2 + 1]]);
            if off == NO_SPRITES_OFFSET {
                continue;
            }
            let off = off as usize;
            let count = *payload.get(off).ok_or_else(|| corrupt("submap offset out of range"))? as usize;
            if count > sizes[submap] as usize {
                return Err(corrupt("submap sprite count exceeds its list size"));
            }
            let mut pos = off + 1;
            for _ in 0..count {
                // Peek the sprite number to learn its extra-byte count.
                let number = *payload.get(pos).ok_or_else(|| corrupt("sprite entry out of range"))? & 0x7F;
                let len = 3 + extra_counts[number as usize] as usize;
                let sprite = CustomOwSprite::decode(
                    payload.get(pos..pos + len).ok_or_else(|| corrupt("sprite entry overruns payload"))?,
                    extra_counts[number as usize] as usize,
                )
                .ok_or_else(|| corrupt("sprite entry decode failed"))?;
                pos += len;
                sprites.push(sprite);
            }
        }
        Ok(table)
    }
}

/// Parse the custom sprite table. Returns `Ok(None)` when the pointer is
/// `FF FF FF` (no table). Returns `Err(SpriteError::ForeignTable)` when the
/// pointer aims at a RATS block smw-editor did not author (e.g. LM's own),
/// which the editor refuses to erase blindly.
pub fn parse_custom_table(rom_bytes: &[u8], header_offset: usize) -> Result<Option<CustomSpriteTable>, SpriteError> {
    let ptr_pc = AddrPc::try_from_lorom(CUSTOM_SPRITE_PTR_SNES)
        .map(|p| p.as_index() + header_offset)
        .map_err(|_| SpriteError::PointerOverrunsRom)?;
    if ptr_pc + 3 > rom_bytes.len() {
        return Err(SpriteError::PointerOverrunsRom);
    }
    let Some(table_snes) = read_snes3(rom_bytes, ptr_pc) else {
        return Ok(None);
    };
    let table_pc = AddrPc::try_from_lorom(table_snes)
        .map(|p| p.as_index() + header_offset)
        .map_err(|_| SpriteError::CustomTableOverrunsRom)?;
    let Some(range) = rats_payload_range(rom_bytes, table_pc) else {
        return Err(SpriteError::ForeignTable);
    };
    let payload = &rom_bytes[range.clone()];
    if payload.len() < CUSTOM_HEADER_LEN || &payload[..8] != CUSTOM_MAGIC {
        return Err(SpriteError::ForeignTable);
    }
    let counts = extra_byte_counts(rom_bytes, header_offset);
    CustomSpriteTable::decode_payload(payload, &counts).map(Some)
}

/// Write the custom sprite table: erase any smw-editor-authored block, then
/// allocate fresh RATS-tagged free space and repoint `$0EF55D`. An empty
/// table erases the block and resets the pointer to `FF FF FF`.
///
/// Refuses (`ForeignTable`) when the existing pointer aims at a block
/// smw-editor did not author.
pub fn write_custom_table(
    table: &CustomSpriteTable, rom_bytes: &mut [u8], header_offset: usize,
) -> Result<(), SpriteError> {
    let ptr_pc = AddrPc::try_from_lorom(CUSTOM_SPRITE_PTR_SNES)?.as_index() + header_offset;
    if ptr_pc + 3 > rom_bytes.len() {
        return Err(SpriteError::PointerOverrunsRom);
    }
    // Erase any existing block first — but only if it's ours.
    if let Some(table_snes) = read_snes3(rom_bytes, ptr_pc) {
        let table_pc = AddrPc::try_from_lorom(table_snes)?.as_index() + header_offset;
        let is_ours = rats_payload_range(rom_bytes, table_pc)
            .map(|range| {
                rom_bytes[range.clone()].len() >= CUSTOM_HEADER_LEN
                    && rom_bytes[range.start..range.start + 8] == *CUSTOM_MAGIC
            })
            .unwrap_or(false);
        if !is_ours {
            return Err(SpriteError::ForeignTable);
        }
        erase_rats_block(rom_bytes, table_pc);
    }

    if table.is_empty() {
        rom_bytes[ptr_pc..ptr_pc + 3].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        return Ok(());
    }

    let counts = extra_byte_counts(rom_bytes, header_offset);
    let payload = table.encode_payload(&counts);
    if payload.is_empty() || payload.len() > 0x10000 {
        return Err(SpriteError::TooLarge(payload.len()));
    }
    let total = 8 + payload.len(); // RATS tag + payload
    let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
        .ok_or(SpriteError::NoFreeSpace(total))?;
    let file_off = pc + header_offset;
    let size_field = (payload.len() - 1) as u16;
    rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
    rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
    rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
    rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);

    let snes = AddrSnes::try_from_lorom(AddrPc(pc as u32))?;
    let snes_bytes = snes.0.to_le_bytes();
    rom_bytes[ptr_pc..ptr_pc + 3].copy_from_slice(&snes_bytes[..3]);
    Ok(())
}

/// Convenience: parse both vanilla sprites and the custom table from a
/// [`Rom`].
pub fn parse_all(
    rom: &Rom, header_offset: usize,
) -> Result<(VanillaOwSprites, Option<CustomSpriteTable>), SpriteError> {
    let bytes = rom.bytes();
    Ok((VanillaOwSprites::parse(bytes, header_offset)?, parse_custom_table(bytes, header_offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal synthetic ROM image: 512 KiB of `0xFF` (free space) with the
    /// vanilla sprite/visibility bytes filled in from the real ROM's values
    /// (verified 2026-09-17).
    fn test_rom() -> Vec<u8> {
        let mut rom = vec![0xFFu8; 0x80000];
        let table_pc = AddrPc::try_from_lorom(VANILLA_SPRITE_TABLE_SNES).unwrap().as_index();
        #[rustfmt::skip]
        let records: [u8; 65] = [
            0x00,0x00,0x01,0xE0,0x00, 0x00,0x00,0x01,0x60,0x00, 0x06,0x70,0x01,0x20,0x00,
            0x07,0x38,0x00,0x8A,0x01, 0x00,0x58,0x00,0x7A,0x00, 0x08,0x88,0x01,0x18,0x00,
            0x09,0x48,0x01,0xFC,0xFF, 0x00,0x80,0x00,0x00,0x01, 0x00,0x50,0x00,0x40,0x01,
            0x03,0x00,0x00,0x00,0x00, 0x0A,0x40,0x00,0x98,0x00, 0x0A,0x60,0x00,0xF8,0x00,
            0x0A,0x40,0x01,0x58,0x01,
        ];
        rom[table_pc..table_pc + 65].copy_from_slice(&records);
        let vis_pc = AddrPc::try_from_lorom(NUMBER0_VISIBILITY_SNES).unwrap().as_index();
        rom[vis_pc] = 0x60; // engine RTS opcode
        #[rustfmt::skip]
        let vis: [u8; 10] = [0x7F,0x21,0x7F,0x7F,0x7F,0x77,0x3F,0xF7,0xF7,0x00];
        rom[vis_pc + 1..vis_pc + 11].copy_from_slice(&vis);
        rom
    }

    #[test]
    fn vanilla_parse_round_trip() {
        let rom = test_rom();
        let parsed = VanillaOwSprites::parse(&rom, 0).unwrap();
        assert_eq!(parsed.sprites.len(), 13);
        assert_eq!(parsed.sprites[0], VanillaOwSprite { number: 0x00, x: 0x0100, y: 0x00E0 });
        assert_eq!(parsed.sprites[5], VanillaOwSprite { number: 0x08, x: 0x0188, y: 0x0018 });
        assert_eq!(parsed.sprites[6].y_px(), -4); // 0xFFFC
        assert_eq!(parsed.sprites[10].type_name(), "Ghost");

        let mut rewritten = rom.clone();
        parsed.write(&mut rewritten, 0).unwrap();
        // Only the 65 + 10 bytes may change; the RTS byte is never written.
        assert_eq!(rewritten[..], rom[..]);
    }

    #[test]
    fn visibility_semantics_match_game_code() {
        // CODE_04F87C: active iff (byte & mask) == 0, byte indexed by number.
        let rom = test_rom();
        let parsed = VanillaOwSprites::parse(&rom, 0).unwrap();
        // Valley of Bowser sign (number 8 -> byte $F7): only Valley of Bowser.
        for submap in 0..7u8 {
            assert_eq!(parsed.is_active_on(0x08, submap), submap == 4, "sign on submap {submap}");
        }
        // Ghosts (number 0x0A -> byte $00): everywhere.
        for submap in 0..7u8 {
            assert!(parsed.is_active_on(0x0A, submap), "ghost on submap {submap}");
        }
        // Yoshi's house smoke (number 7 -> byte $3F): main map yes, Vanilla Dome no.
        assert!(parsed.is_active_on(0x07, 0));
        assert!(!parsed.is_active_on(0x07, 2));
        // Number 0 reads the $60 RTS opcode: active main, inactive Yoshi's.
        assert_eq!(parsed.visibility[0], 0x60);
        assert!(parsed.is_active_on(0x00, 0));
        assert!(!parsed.is_active_on(0x00, 1));
    }

    #[test]
    fn visibility_edit_round_trip() {
        let rom = test_rom();
        let mut parsed = VanillaOwSprites::parse(&rom, 0).unwrap();
        // Deactivate the sign on Valley of Bowser.
        parsed.set_active_on(0x08, 4, false).unwrap();
        assert!(!parsed.is_active_on(0x08, 4));
        // Reactivate.
        parsed.set_active_on(0x08, 4, true).unwrap();
        assert!(parsed.is_active_on(0x08, 4));
        // Number 0 is not editable (would clobber the engine RTS).
        assert!(parsed.set_active_on(0x00, 0, false).is_err());
        // Out-of-range numbers are rejected.
        assert!(parsed.set_active_on(0x0B, 0, false).is_err());

        let mut rewritten = rom.clone();
        parsed.write(&mut rewritten, 0).unwrap();
        assert_eq!(rewritten, rom);
    }

    #[test]
    fn custom_entry_bit_layout_round_trip() {
        // xnnnnnnn yyyXXXXX hhhhhYYY eeeeeeee: 6-bit X/Y in 8px units,
        // 7-bit number, 5-bit height.
        let sprite = CustomOwSprite { number: 0x4A, x: 0x2A, y: 0x15, height: 0x0C, extra: vec![0xEE] };
        let encoded = sprite.encode(1);
        assert_eq!(encoded.len(), 4);
        let decoded = CustomOwSprite::decode(&encoded, 1).unwrap();
        assert_eq!(decoded, sprite);
        assert_eq!(decoded.x_px(), 0x2A * 8);
        assert_eq!(decoded.y_px(), 0x15 * 8);
        // X bit 5 rides in byte 0 bit 7.
        assert_eq!(encoded[0], 0x80 | 0x4A);
        assert_eq!(encoded[1], ((0x15 >> 3) << 5) | (0x2A & 0x1F));
        assert_eq!(encoded[2], (0x0C << 3) | (0x15 & 0x07));
        assert_eq!(encoded[3], 0xEE);
    }

    #[test]
    fn custom_table_payload_round_trip() {
        let mut table = CustomSpriteTable::default();
        table.submaps[0].push(CustomOwSprite { number: 1, x: 10, y: 20, height: 3, extra: vec![0xAA] });
        table.submaps[4].push(CustomOwSprite { number: 0x7F, x: 63, y: 63, height: 31, extra: vec![0x01] });
        let counts = [1u8; 128];
        let payload = table.encode_payload(&counts);
        let decoded = CustomSpriteTable::decode_payload(&payload, &counts).unwrap();
        assert_eq!(decoded, table);
        // Empty submaps decode back to empty.
        assert!(decoded.submaps[1].is_empty());
        assert_eq!(decoded.total_count(), 2);
    }

    #[test]
    fn custom_table_rom_write_read_erase() {
        let mut rom = test_rom();
        let mut table = CustomSpriteTable::default();
        table.submaps[2].push(CustomOwSprite { number: 5, x: 1, y: 2, height: 1, extra: vec![0x00] });

        // No table yet.
        assert_eq!(parse_custom_table(&rom, 0).unwrap(), None);

        write_custom_table(&table, &mut rom, 0).unwrap();
        let parsed = parse_custom_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(parsed, table);

        // Pointer no longer FF FF FF and aims at a STAR tag.
        let ptr_pc = AddrPc::try_from_lorom(CUSTOM_SPRITE_PTR_SNES).unwrap().as_index();
        assert_ne!(&rom[ptr_pc..ptr_pc + 3], &[0xFF, 0xFF, 0xFF]);
        let snes = AddrSnes(u32::from_le_bytes([rom[ptr_pc], rom[ptr_pc + 1], rom[ptr_pc + 2], 0]));
        let tag_pc = AddrPc::try_from_lorom(snes).unwrap().as_index();
        assert_eq!(&rom[tag_pc..tag_pc + 4], b"STAR");

        // Erasing: empty table removes the block and resets the pointer.
        write_custom_table(&CustomSpriteTable::default(), &mut rom, 0).unwrap();
        assert_eq!(parse_custom_table(&rom, 0).unwrap(), None);
        assert_eq!(&rom[ptr_pc..ptr_pc + 3], &[0xFF, 0xFF, 0xFF]);
        assert_eq!(&rom[tag_pc..tag_pc + 4], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn list_sizes_encode_decode_round_trip() {
        let mut table = CustomSpriteTable::default();
        table.set_list_size(0, 8).unwrap();
        table.set_list_size(4, 0).unwrap();
        table.submaps[0].push(CustomOwSprite { number: 1, x: 10, y: 20, height: 3, extra: vec![0xAA] });
        table.submaps[0].push(CustomOwSprite { number: 2, x: 11, y: 21, height: 0, extra: vec![0xBB] });
        let counts = [1u8; 128];
        let payload = table.encode_payload(&counts);
        assert_eq!(payload[8], CUSTOM_VERSION);
        let decoded = CustomSpriteTable::decode_payload(&payload, &counts).unwrap();
        assert_eq!(decoded, table);
        assert_eq!(decoded.list_size(0), 8);
        assert_eq!(decoded.list_size(4), 0);
        assert_eq!(decoded.list_size(1), DEFAULT_CUSTOM_LIST_SIZE);
        assert_eq!(decoded.room_for(0), 6);
    }

    #[test]
    fn v1_payload_decodes_with_default_list_sizes() {
        // Version-1 payload: 23-byte header, no size bytes. Must load with
        // every submap defaulted to 24 (backward compat with older saves).
        let counts = [1u8; 128];
        let mut v1 = Vec::new();
        v1.extend_from_slice(b"OWSPRITE");
        v1.push(1u8);
        let list_off: u16 = 23;
        v1.extend_from_slice(&list_off.to_le_bytes());
        for _ in 0..6 {
            v1.extend_from_slice(&NO_SPRITES_OFFSET.to_le_bytes());
        }
        let sprite = CustomOwSprite { number: 5, x: 1, y: 2, height: 1, extra: vec![0x00] };
        v1.push(1u8);
        v1.extend_from_slice(&sprite.encode(1));
        let decoded = CustomSpriteTable::decode_payload(&v1, &counts).unwrap();
        assert_eq!(decoded.list_sizes, [DEFAULT_CUSTOM_LIST_SIZE; 7]);
        assert_eq!(decoded.submaps[0].len(), 1);
        assert_eq!(decoded.submaps[0][0].number, 5);
    }

    #[test]
    fn set_list_size_validation() {
        let mut table = CustomSpriteTable::default();
        table.submaps[2].push(CustomOwSprite { number: 5, x: 1, y: 2, height: 1, extra: vec![0x00] });
        table.submaps[2].push(CustomOwSprite { number: 6, x: 2, y: 3, height: 1, extra: vec![0x00] });
        // Cannot shrink below the current sprite count.
        assert!(matches!(
            table.set_list_size(2, 1),
            Err(SpriteError::ListSizeTooSmall { submap: 2, size: 1, count: 2 })
        ));
        assert_eq!(table.list_size(2), DEFAULT_CUSTOM_LIST_SIZE);
        // Exactly the count is fine.
        table.set_list_size(2, 2).unwrap();
        assert_eq!(table.list_size(2), 2);
        assert_eq!(table.room_for(2), 0);
        // Sizes clamp to the native maximum of 24.
        table.set_list_size(3, 255).unwrap();
        assert_eq!(table.list_size(3), MAX_CUSTOM_SPRITES_PER_SUBMAP as u8);
        // Out-of-range submaps are inert.
        assert!(table.set_list_size(7, 5).is_ok());
        assert_eq!(table.list_size(7), 0);
    }

    #[test]
    fn encode_truncates_lists_to_configured_size() {
        let mut table = CustomSpriteTable::default();
        table.set_list_size(1, 3).unwrap();
        for i in 0..5u8 {
            table.submaps[1].push(CustomOwSprite { number: i, x: i, y: 1, height: 1, extra: vec![0x00] });
        }
        let counts = [1u8; 128];
        let payload = table.encode_payload(&counts);
        let decoded = CustomSpriteTable::decode_payload(&payload, &counts).unwrap();
        // Only the first 3 survive the round trip; the rest were truncated.
        assert_eq!(decoded.submaps[1].len(), 3);
        assert_eq!(decoded.submaps[1][2].number, 2);
        assert_eq!(decoded.list_size(1), 3);
    }

    #[test]
    fn decode_rejects_count_above_list_size() {
        let counts = [1u8; 128];
        let mut table = CustomSpriteTable::default();
        table.set_list_size(0, 1).unwrap();
        table.submaps[0].push(CustomOwSprite { number: 1, x: 1, y: 1, height: 1, extra: vec![0x00] });
        let mut payload = table.encode_payload(&counts);
        // Corrupt the count byte of submap 0's list to exceed its size.
        let off = u16::from_le_bytes([payload[9], payload[10]]) as usize;
        payload[off] = 5;
        assert!(matches!(CustomSpriteTable::decode_payload(&payload, &counts), Err(SpriteError::Corrupt(_))));
    }

    #[test]
    fn foreign_table_is_not_touched() {
        let mut rom = test_rom();
        // Plant a foreign (non-smw-editor) RATS block and point at it.
        let tag_pc = 0x10000;
        rom[tag_pc..tag_pc + 4].copy_from_slice(b"STAR");
        rom[tag_pc + 4..tag_pc + 6].copy_from_slice(&31u16.to_le_bytes());
        rom[tag_pc + 6..tag_pc + 8].copy_from_slice(&(!31u16).to_le_bytes());
        rom[tag_pc + 8..tag_pc + 16].copy_from_slice(b"NOTOURS!");
        let snes = AddrSnes::try_from_lorom(AddrPc(tag_pc as u32)).unwrap();
        let ptr_pc = AddrPc::try_from_lorom(CUSTOM_SPRITE_PTR_SNES).unwrap().as_index();
        rom[ptr_pc..ptr_pc + 3].copy_from_slice(&snes.0.to_le_bytes()[..3]);

        assert!(matches!(parse_custom_table(&rom, 0), Err(SpriteError::ForeignTable)));
        let mut table = CustomSpriteTable::default();
        table.submaps[0].push(CustomOwSprite { number: 1, x: 1, y: 1, height: 1, extra: vec![0] });
        assert!(matches!(write_custom_table(&table, &mut rom, 0), Err(SpriteError::ForeignTable)));
        // Foreign block untouched.
        assert_eq!(&rom[tag_pc..tag_pc + 4], b"STAR");
    }

    #[test]
    fn extra_byte_counts_default_and_marker() {
        let rom = test_rom();
        assert_eq!(extra_byte_counts(&rom, 0)[0x10], 1);

        // Install the LM v3.51 marker + table.
        let mut rom = rom;
        let ptr_pc = AddrPc::try_from_lorom(EXTRA_BYTE_COUNT_PTR_SNES).unwrap().as_index();
        let marker_pc = AddrPc::try_from_lorom(EXTRA_BYTE_COUNT_MARKER_SNES).unwrap().as_index();
        let table_pc = 0x20000;
        let snes = AddrSnes::try_from_lorom(AddrPc(table_pc as u32)).unwrap();
        rom[ptr_pc..ptr_pc + 3].copy_from_slice(&snes.0.to_le_bytes()[..3]);
        rom[marker_pc] = EXTRA_BYTE_COUNT_MARKER;
        rom[table_pc..table_pc + 128].fill(1);
        rom[table_pc + 0x10] = 5;
        let counts = extra_byte_counts(&rom, 0);
        assert_eq!(counts[0x10], 5);
        assert_eq!(counts[0x11], 1);
    }

    /// Real-ROM validation: parse the vanilla sprite/visibility tables from
    /// an actual SMW ROM and check the values verified against SMWDisX
    /// `bank_04.asm` on 2026-09-17. Requires `ROM_PATH`.
    #[test]
    #[ignore]
    fn real_rom_vanilla_sprites() {
        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must be set");
        let raw = std::fs::read(&rom_path).expect("read rom");
        let (bytes, header_offset) = if raw.len() % 0x400 == 0x200 { (raw[0x200..].to_vec(), 0x200) } else { (raw, 0) };
        let parsed = VanillaOwSprites::parse(&bytes, header_offset).unwrap();

        // Slot 0: number 0x00 at (0x0100, 0x00E0).
        assert_eq!(parsed.sprites[0], VanillaOwSprite { number: 0x00, x: 0x0100, y: 0x00E0 });
        // Slot 5: Valley of Bowser sign (number 0x08) at (0x0188, 0x0018).
        assert_eq!(parsed.sprites[5], VanillaOwSprite { number: 0x08, x: 0x0188, y: 0x0018 });
        // Slot 6: sprite 0x09 at y = 0xFFFC (-4 px).
        assert_eq!(parsed.sprites[6].number, 0x09);
        assert_eq!(parsed.sprites[6].y_px(), -4);
        // Slots 10-12: ghosts (number 0x0A).
        for slot in 10..13 {
            assert_eq!(parsed.sprites[slot].number, 0x0A);
        }

        // Visibility bytes for numbers 1..=10, from ROM $04F829.
        assert_eq!(parsed.visibility, [0x60, 0x7F, 0x21, 0x7F, 0x7F, 0x7F, 0x77, 0x3F, 0xF7, 0xF7, 0x00]);
        // Game-code semantics (CODE_04F87C): the sign is Valley-of-Bowser-only.
        for submap in 0..7u8 {
            assert_eq!(parsed.is_active_on(0x08, submap), submap == 4);
        }

        // Vanilla ROM has no custom sprite table (pointer is FF FF FF).
        assert_eq!(parse_custom_table(&bytes, header_offset).unwrap(), None);
        // ... and no extra-byte-count table either.
        assert!(extra_byte_counts(&bytes, header_offset).iter().all(|&c| c == 1));
    }
}
