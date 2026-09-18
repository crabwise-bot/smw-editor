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
//! then a variable number of extra bytes (default 1; LM v3.51+ may provide a
//! per-sprite record-size table — see "Custom overworld sprite record sizes"
//! below — whose 3-byte pointer lives at `$0DE18C` with marker byte `$42` at
//! `$0DE18F`).
//!
//! ## Custom overworld sprite record sizes (LM v3.51)
//!
//! Lunar Magic 3.51 (2024-12-25) added support for a user-defined
//! *sprite size table* for custom overworld sprites: one byte per sprite
//! number giving the **total** number of bytes that sprite's records take up
//! in the custom sprite list (3 = the fixed bytes only, no extra bytes;
//! `0xF` = maximum; default 4 when no table is defined). Per LM's own help
//! file ("Custom Overworld Sprite List Sizes", verified against the LM 3.63
//! download 2026-09-18): the table is `0x7F` bytes with the first entry for
//! sprite 1 (in 3.51 only it was `0x80` bytes starting at sprite 0; 3.60
//! changed it and disallowed inserting custom sprite 0), the table's SNES
//! address goes in the 3 bytes at `$0DE18C` (PC `0x6E38C` in a headered ROM),
//! and `$0DE18F` (PC `0x6E38F`) must hold `$42` to enable it.
//!
//! smw-editor models this as [`SpriteSizeTable`] — 0x7F total record sizes
//! for sprites `1..=0x7F`, each `3..=0xF`. The per-sprite *extra*-byte counts
//! used to encode/decode custom sprite records are derived from it
//! (`extra = size - 3`; see [`extra_byte_counts`]).
//!
//! **Ownership:** Lunar Magic itself almost never authors this table —
//! "typically the sizes would be set by a 3rd party utility" (LM help), the
//! one exception being overworld transfer to another ROM. So the table is
//! LM/3rd-party-owned data: if the ROM already has one (marker `$42`
//! present), the editor edits its bytes in place; if not, the editor can
//! create one in free space (RATS-tagged so it can be found again), point
//! `$0DE18C` at it, and set the marker — after which Lunar Magic reads it
//! like any 3rd-party table.
//!
//! **Important:** the table only tells *Lunar Magic* how to parse the custom
//! sprite list. "It's up to you or a 3rd party utility to modify the game
//! code to take the sizes into account" (LM help) — like custom sprites
//! themselves, the sizes do nothing in-game without a runtime patch.
//!
//! LM's exact on-disk layout for the *custom sprite* table is not publicly
//! documented, so smw-editor uses its own clearly-marked RATS-tagged format
//! (magic `OWSPRITE`, version 1): the 3-byte pointer at `$0EF55D` aims at the
//! `STAR` tag, and the seven submap offsets are byte offsets from the start
//! of the RATS payload (`0xFFFF` = submap has no custom sprites).

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
/// 3-byte SNES pointer (little-endian) to the 0x7F-entry sprite record-size
/// table (LM v3.51+). The pointer aims at the first size byte.
pub const SIZE_TABLE_PTR_SNES: AddrSnes = AddrSnes(0x0DE18C);
/// Marker byte address: when this holds [`SIZE_TABLE_MARKER`], the
/// record-size table is present and enabled for Lunar Magic.
pub const SIZE_TABLE_MARKER_SNES: AddrSnes = AddrSnes(0x0DE18F);
/// Marker byte value enabling the record-size table.
pub const SIZE_TABLE_MARKER: u8 = 0x42;
/// Entries in the record-size table: one per sprite number `1..=0x7F`.
/// (LM 3.51 used 0x80 entries starting at sprite 0; LM 3.60 changed it to
/// 0x7F entries starting at sprite 1.)
pub const SIZE_TABLE_LEN: usize = 0x7F;
/// Minimum total record size: the 3 fixed bytes, no extra bytes.
pub const MIN_SPRITE_RECORD_SIZE: u8 = 3;
/// Maximum total record size: 3 fixed bytes + 12 extra bytes.
pub const MAX_SPRITE_RECORD_SIZE: u8 = 0xF;
/// Record size assumed when the ROM has no size table: 3 fixed + 1 extra
/// byte (matches LM's own default of 4).
pub const DEFAULT_SPRITE_RECORD_SIZE: u8 = 4;
/// Default extra bytes per custom sprite when no size table is present.
pub const DEFAULT_EXTRA_BYTES: usize = 1;
/// Maximum custom sprites per submap: the documented native LM limit
/// (smwspeedruns "Overworld Data Format", accurate as of LM 3.51).
pub const MAX_CUSTOM_SPRITES_PER_SUBMAP: usize = 24;

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
    #[error("sprite {number:#04X}: record size {size} out of range (3..=0xF)")]
    BadSpriteSize { number: u8, size: u8 },
    #[error("this ROM has no custom overworld sprite size table")]
    NoSizeTable,
    #[error("this ROM already has a custom overworld sprite size table")]
    SizeTableExists,
    #[error("sprite size table overruns ROM")]
    SizeTableOverrunsRom,
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomSpriteTable {
    pub submaps: [Vec<CustomOwSprite>; 7],
}

impl CustomSpriteTable {
    pub fn is_empty(&self) -> bool {
        self.submaps.iter().all(Vec::is_empty)
    }

    pub fn total_count(&self) -> usize {
        self.submaps.iter().map(Vec::len).sum()
    }
}

// -------------------------------------------------------------------------------------------------
// Custom table ROM storage (smw-editor RATS format)
// -------------------------------------------------------------------------------------------------

/// Magic at the start of our custom-sprite RATS payload.
const CUSTOM_MAGIC: &[u8; 8] = b"OWSPRITE";
const CUSTOM_VERSION: u8 = 1;
/// Payload header: magic + version + 7 submap offsets.
const CUSTOM_HEADER_LEN: usize = 8 + 1 + 7 * 2;
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

/// Per-sprite extra-byte counts for custom sprite numbers `0..=0x7F`.
///
/// When LM v3.51+'s record-size table is present (marker `$42` at
/// [`SIZE_TABLE_MARKER_SNES`]), each sprite number `1..=0x7F` gets
/// `table_size - 3` extra bytes (the table stores *total* record sizes per
/// LM's help file: 3 = no extra bytes, `0xF` = max). Sprite 0 has no table
/// entry and always uses the default. Otherwise every sprite number uses
/// [`DEFAULT_EXTRA_BYTES`] (total record size 4 = LM's default).
pub fn extra_byte_counts(rom_bytes: &[u8], header_offset: usize) -> [u8; 128] {
    let mut counts = [DEFAULT_EXTRA_BYTES as u8; 128];
    if let Ok(Some(table)) = parse_size_table(rom_bytes, header_offset) {
        for (i, size) in table.sizes.iter().enumerate() {
            counts[i + 1] = size.saturating_sub(MIN_SPRITE_RECORD_SIZE);
        }
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
    /// Encode to the RATS payload format (without the `STAR` tag).
    pub fn encode_payload(&self, extra_counts: &[u8; 128]) -> Vec<u8> {
        let mut lists: Vec<Vec<u8>> = Vec::with_capacity(7);
        for sprites in &self.submaps {
            let mut list = Vec::with_capacity(1 + sprites.len() * 4);
            list.push(sprites.len().min(MAX_CUSTOM_SPRITES_PER_SUBMAP) as u8);
            for sprite in sprites.iter().take(MAX_CUSTOM_SPRITES_PER_SUBMAP) {
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
        for list in &lists {
            if list.len() > 1 {
                out.extend_from_slice(list);
            }
        }
        out
    }

    /// Decode a RATS payload produced by [`Self::encode_payload`].
    pub fn decode_payload(payload: &[u8], extra_counts: &[u8; 128]) -> Result<Self, SpriteError> {
        let corrupt = |msg: &str| SpriteError::Corrupt(msg.to_string());
        if payload.len() < CUSTOM_HEADER_LEN {
            return Err(corrupt("payload shorter than header"));
        }
        if &payload[..8] != CUSTOM_MAGIC {
            return Err(corrupt("bad magic"));
        }
        if payload[8] != CUSTOM_VERSION {
            return Err(SpriteError::Corrupt(format!("unsupported version {}", payload[8])));
        }
        let mut table = CustomSpriteTable::default();
        for (submap, sprites) in table.submaps.iter_mut().enumerate() {
            let off = u16::from_le_bytes([payload[9 + submap * 2], payload[9 + submap * 2 + 1]]);
            if off == NO_SPRITES_OFFSET {
                continue;
            }
            let off = off as usize;
            let count = *payload.get(off).ok_or_else(|| corrupt("submap offset out of range"))? as usize;
            if count > MAX_CUSTOM_SPRITES_PER_SUBMAP {
                return Err(corrupt("submap sprite count exceeds 24"));
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

// -------------------------------------------------------------------------------------------------
// Sprite record-size table (LM v3.51+)
// -------------------------------------------------------------------------------------------------

/// The LM v3.51+ custom overworld sprite record-size table: one **total**
/// record size per sprite number `1..=0x7F`, each `3..=0xF` (3 = the fixed
/// bytes only, no extra bytes; `0xF` = maximum; LM's default when no table
/// exists is 4).
///
/// Table index `i` is the size for sprite number `i + 1` (LM 3.60+ layout;
/// LM 3.51 used 0x80 entries starting at sprite 0 — see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSizeTable {
    pub sizes: [u8; SIZE_TABLE_LEN],
}

impl Default for SpriteSizeTable {
    fn default() -> Self {
        Self { sizes: [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN] }
    }
}

impl SpriteSizeTable {
    /// Total record size for sprite `number`: the table entry for
    /// `1..=0x7F`, [`DEFAULT_SPRITE_RECORD_SIZE`] for anything else
    /// (sprite 0 has no entry).
    pub fn size_for(&self, number: u8) -> u8 {
        if (1..=0x7F).contains(&number) {
            self.sizes[(number - 1) as usize]
        } else {
            DEFAULT_SPRITE_RECORD_SIZE
        }
    }

    /// Extra-byte count for sprite `number` (`size - 3`).
    pub fn extra_for(&self, number: u8) -> u8 {
        self.size_for(number).saturating_sub(MIN_SPRITE_RECORD_SIZE)
    }

    /// Set the record size for sprite `number` (`1..=0x7F`). Sizes outside
    /// `3..=0xF` are refused with [`SpriteError::BadSpriteSize`].
    pub fn set_size(&mut self, number: u8, size: u8) -> Result<(), SpriteError> {
        if !(1..=0x7Fu8).contains(&number) {
            return Err(SpriteError::BadSpriteSize { number, size });
        }
        if !(MIN_SPRITE_RECORD_SIZE..=MAX_SPRITE_RECORD_SIZE).contains(&size) {
            return Err(SpriteError::BadSpriteSize { number, size });
        }
        self.sizes[(number - 1) as usize] = size;
        Ok(())
    }

    /// Encode the raw 0x7F-byte table (no RATS tag — Lunar Magic reads the
    /// bytes directly from the `$0DE18C` pointer).
    pub fn encode_table(&self) -> [u8; SIZE_TABLE_LEN] {
        self.sizes
    }
}

/// Parse the sprite record-size table. Returns `Ok(None)` when the ROM has
/// no table (marker byte not `$42`, or the pointer is `FF FF FF`).
///
/// Out-of-range bytes are clamped to `3..=0xF`: the table is
/// LM/3rd-party-owned data, and one odd byte should not nuke the overworld
/// tab. The editor normalizes values on write.
pub fn parse_size_table(rom_bytes: &[u8], header_offset: usize) -> Result<Option<SpriteSizeTable>, SpriteError> {
    let ptr_pc = AddrPc::try_from_lorom(SIZE_TABLE_PTR_SNES)
        .map(|p| p.as_index() + header_offset)
        .map_err(|_| SpriteError::SizeTableOverrunsRom)?;
    let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES)
        .map(|p| p.as_index() + header_offset)
        .map_err(|_| SpriteError::SizeTableOverrunsRom)?;
    if ptr_pc + 3 > rom_bytes.len() || marker_pc >= rom_bytes.len() {
        return Err(SpriteError::SizeTableOverrunsRom);
    }
    if rom_bytes[marker_pc] != SIZE_TABLE_MARKER {
        return Ok(None);
    }
    let Some(table_snes) = read_snes3(rom_bytes, ptr_pc) else {
        return Ok(None);
    };
    let table_pc = AddrPc::try_from_lorom(table_snes)
        .map(|p| p.as_index() + header_offset)
        .map_err(|_| SpriteError::SizeTableOverrunsRom)?;
    let raw = rom_bytes.get(table_pc..table_pc + SIZE_TABLE_LEN).ok_or(SpriteError::SizeTableOverrunsRom)?;
    let mut sizes = [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN];
    for (i, b) in raw.iter().enumerate() {
        sizes[i] = (*b).clamp(MIN_SPRITE_RECORD_SIZE, MAX_SPRITE_RECORD_SIZE);
    }
    Ok(Some(SpriteSizeTable { sizes }))
}

/// Overwrite the ROM's existing size table in place. Works for tables
/// authored by Lunar Magic / 3rd-party utilities (raw 0x7F bytes) and for
/// tables smw-editor created itself (RATS-tagged; the pointer aims past the
/// tag at the data). Returns [`SpriteError::NoSizeTable`] when the ROM has
/// no table — use [`create_size_table`] instead.
pub fn write_size_table(
    table: &SpriteSizeTable, rom_bytes: &mut [u8], header_offset: usize,
) -> Result<(), SpriteError> {
    let ptr_pc = AddrPc::try_from_lorom(SIZE_TABLE_PTR_SNES)?.as_index() + header_offset;
    let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES)?.as_index() + header_offset;
    if ptr_pc + 3 > rom_bytes.len() || marker_pc >= rom_bytes.len() {
        return Err(SpriteError::SizeTableOverrunsRom);
    }
    if rom_bytes[marker_pc] != SIZE_TABLE_MARKER {
        return Err(SpriteError::NoSizeTable);
    }
    let Some(table_snes) = read_snes3(rom_bytes, ptr_pc) else {
        return Err(SpriteError::NoSizeTable);
    };
    let table_pc = AddrPc::try_from_lorom(table_snes)?.as_index() + header_offset;
    let dst = rom_bytes.get_mut(table_pc..table_pc + SIZE_TABLE_LEN).ok_or(SpriteError::SizeTableOverrunsRom)?;
    dst.copy_from_slice(&table.encode_table());
    Ok(())
}

/// Create a sprite record-size table in free space and enable it: allocate
/// a RATS-tagged block holding the 0x7F size bytes, point `$0DE18C` at the
/// data (past the tag), and set the `$42` marker at `$0DE18F`. Lunar Magic
/// reads the table like any 3rd-party-authored one.
///
/// Returns [`SpriteError::SizeTableExists`] when the ROM already has a
/// table — edit it with [`write_size_table`] instead.
pub fn create_size_table(
    table: &SpriteSizeTable, rom_bytes: &mut [u8], header_offset: usize,
) -> Result<(), SpriteError> {
    if parse_size_table(rom_bytes, header_offset)?.is_some() {
        return Err(SpriteError::SizeTableExists);
    }
    let total = 8 + SIZE_TABLE_LEN; // RATS tag + 0x7F size bytes
    let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
        .ok_or(SpriteError::NoFreeSpace(total))?;
    let file_off = pc + header_offset;
    let size_field = (SIZE_TABLE_LEN - 1) as u16;
    rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
    rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
    rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
    rom_bytes[file_off + 8..file_off + 8 + SIZE_TABLE_LEN].copy_from_slice(&table.encode_table());

    // The pointer aims at the size bytes, past the RATS tag.
    let snes = AddrSnes::try_from_lorom(AddrPc(pc as u32 + 8))?;
    let ptr_pc = AddrPc::try_from_lorom(SIZE_TABLE_PTR_SNES)?.as_index() + header_offset;
    let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES)?.as_index() + header_offset;
    if ptr_pc + 3 > rom_bytes.len() || marker_pc >= rom_bytes.len() {
        return Err(SpriteError::SizeTableOverrunsRom);
    }
    rom_bytes[ptr_pc..ptr_pc + 3].copy_from_slice(&snes.0.to_le_bytes()[..3]);
    rom_bytes[marker_pc] = SIZE_TABLE_MARKER;
    Ok(())
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
    fn size_table_default_and_validation() {
        let table = SpriteSizeTable::default();
        assert_eq!(table.sizes, [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN]);
        assert_eq!(table.size_for(1), 4);
        assert_eq!(table.size_for(0x7F), 4);
        // Sprite 0 has no entry: always the default.
        assert_eq!(table.size_for(0), DEFAULT_SPRITE_RECORD_SIZE);
        assert_eq!(table.extra_for(1), 1);

        let mut table = table;
        table.set_size(1, 3).unwrap();
        table.set_size(0x7F, 0xF).unwrap();
        assert_eq!(table.size_for(1), 3);
        assert_eq!(table.extra_for(1), 0);
        assert_eq!(table.size_for(0x7F), 0xF);
        assert_eq!(table.extra_for(0x7F), 12);
        // Out-of-range sizes and sprite numbers are refused.
        assert!(table.set_size(1, 2).is_err());
        assert!(table.set_size(1, 0x10).is_err());
        assert!(table.set_size(0, 4).is_err());
        assert!(table.set_size(0x80, 4).is_err());
        // Failed sets leave the table unchanged.
        assert_eq!(table.size_for(1), 3);
    }

    /// Helper: install a fake "LM-authored" raw 0x7F-byte size table (no
    /// RATS tag, like a 3rd-party utility would write) and enable it.
    fn install_raw_size_table(rom: &mut [u8], sizes: &[u8; SIZE_TABLE_LEN]) {
        let ptr_pc = AddrPc::try_from_lorom(SIZE_TABLE_PTR_SNES).unwrap().as_index();
        let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES).unwrap().as_index();
        let table_pc = 0x20000;
        let snes = AddrSnes::try_from_lorom(AddrPc(table_pc as u32)).unwrap();
        rom[ptr_pc..ptr_pc + 3].copy_from_slice(&snes.0.to_le_bytes()[..3]);
        rom[marker_pc] = SIZE_TABLE_MARKER;
        rom[table_pc..table_pc + SIZE_TABLE_LEN].copy_from_slice(sizes);
    }

    #[test]
    fn size_table_create_parse_write_round_trip() {
        let mut rom = test_rom();
        // Vanilla-ish ROM: no table.
        assert_eq!(parse_size_table(&rom, 0).unwrap(), None);

        // Create: allocates a RATS-tagged block, points $0DE18C at the data,
        // sets the $42 marker.
        let mut table = SpriteSizeTable::default();
        table.set_size(0x10, 7).unwrap();
        create_size_table(&table, &mut rom, 0).unwrap();

        let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES).unwrap().as_index();
        assert_eq!(rom[marker_pc], SIZE_TABLE_MARKER);
        let ptr_pc = AddrPc::try_from_lorom(SIZE_TABLE_PTR_SNES).unwrap().as_index();
        let snes = AddrSnes(u32::from_le_bytes([rom[ptr_pc], rom[ptr_pc + 1], rom[ptr_pc + 2], 0]));
        let data_pc = AddrPc::try_from_lorom(snes).unwrap().as_index();
        // RATS tag immediately before the data the pointer aims at.
        assert_eq!(&rom[data_pc - 8..data_pc - 4], b"STAR");

        let parsed = parse_size_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(parsed, table);
        assert_eq!(parsed.size_for(0x10), 7);

        // Edit in place.
        let mut edited = parsed;
        edited.set_size(0x10, 3).unwrap();
        edited.set_size(0x7F, 0xF).unwrap();
        write_size_table(&edited, &mut rom, 0).unwrap();
        let reparsed = parse_size_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(reparsed, edited);

        // Creating again is refused; the existing table is untouched.
        assert!(matches!(create_size_table(&table, &mut rom, 0), Err(SpriteError::SizeTableExists)));
        assert_eq!(parse_size_table(&rom, 0).unwrap().unwrap(), edited);
    }

    #[test]
    fn write_size_table_without_table_errors() {
        let mut rom = test_rom();
        let table = SpriteSizeTable::default();
        assert!(matches!(write_size_table(&table, &mut rom, 0), Err(SpriteError::NoSizeTable)));
        // Marker set but pointer FF FF FF: still no table.
        let marker_pc = AddrPc::try_from_lorom(SIZE_TABLE_MARKER_SNES).unwrap().as_index();
        rom[marker_pc] = SIZE_TABLE_MARKER;
        assert!(matches!(write_size_table(&table, &mut rom, 0), Err(SpriteError::NoSizeTable)));
        assert_eq!(parse_size_table(&rom, 0).unwrap(), None);
    }

    #[test]
    fn size_table_lm_authored_raw_table() {
        // A 3rd-party utility's raw table (no RATS tag): parse and in-place
        // write must work on it.
        let mut rom = test_rom();
        let mut sizes = [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN];
        sizes[0] = 5; // sprite 1: 5 total bytes = 2 extra
        sizes[0x7E] = 0xF; // sprite 0x7F: max
        install_raw_size_table(&mut rom, &sizes);

        let parsed = parse_size_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(parsed.size_for(1), 5);
        assert_eq!(parsed.size_for(0x7F), 0xF);
        assert_eq!(parsed.size_for(2), 4);

        let mut edited = parsed;
        edited.set_size(1, 3).unwrap();
        write_size_table(&edited, &mut rom, 0).unwrap();
        // Still a raw table (no RATS tag introduced), first byte updated.
        let table_pc = 0x20000;
        assert_eq!(rom[table_pc], 3);
        assert_eq!(&rom[table_pc - 8..table_pc - 4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(parse_size_table(&rom, 0).unwrap().unwrap(), edited);
    }

    #[test]
    fn size_table_clamps_out_of_range_bytes_on_read() {
        let mut rom = test_rom();
        let mut sizes = [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN];
        sizes[0] = 0x02; // below min 3
        sizes[1] = 0x20; // above max 0xF
        install_raw_size_table(&mut rom, &sizes);
        let parsed = parse_size_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(parsed.size_for(1), MIN_SPRITE_RECORD_SIZE);
        assert_eq!(parsed.size_for(2), MAX_SPRITE_RECORD_SIZE);
    }

    #[test]
    fn custom_payload_uses_size_table_record_lengths() {
        // End to end: with a size table giving sprite 5 a 6-byte record
        // (3 extra), a sprite-5 entry round-trips its 3 extra bytes through
        // write_custom_table / parse_custom_table.
        let mut rom = test_rom();
        let mut sizes = SpriteSizeTable::default();
        sizes.set_size(5, 6).unwrap();
        create_size_table(&sizes, &mut rom, 0).unwrap();

        let counts = extra_byte_counts(&rom, 0);
        assert_eq!(counts[5], 3);
        let mut table = CustomSpriteTable::default();
        table.submaps[2].push(CustomOwSprite {
            number: 5,
            x:      1,
            y:      2,
            height: 1,
            extra:  vec![0xAA, 0xBB, 0xCC],
        });
        write_custom_table(&table, &mut rom, 0).unwrap();
        let parsed = parse_custom_table(&rom, 0).unwrap().expect("table should exist");
        assert_eq!(parsed.submaps[2][0].extra, vec![0xAA, 0xBB, 0xCC]);
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
        assert!(extra_byte_counts(&rom, 0).iter().all(|&c| c == 1));

        // Install the LM v3.51+ marker + table. Table bytes are TOTAL record
        // sizes (LM help: 3 = no extra bytes, 0xF = max); entry i is sprite
        // i+1 (LM 3.60+ layout).
        let mut rom = rom;
        let mut sizes = [DEFAULT_SPRITE_RECORD_SIZE; SIZE_TABLE_LEN];
        sizes[0x0F] = 6; // sprite 0x10: 6 total = 3 extra
        sizes[0x7E] = 3; // sprite 0x7F: 3 total = 0 extra
        install_raw_size_table(&mut rom, &sizes);
        let counts = extra_byte_counts(&rom, 0);
        assert_eq!(counts[0x10], 3);
        assert_eq!(counts[0x7F], 0);
        assert_eq!(counts[0x11], 1);
        // Sprite 0 has no table entry: always the default.
        assert_eq!(counts[0], 1);
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
        // ... and no sprite record-size table either.
        assert_eq!(parse_size_table(&bytes, header_offset).unwrap(), None);
        assert!(extra_byte_counts(&bytes, header_offset).iter().all(|&c| c == 1));
    }
}
