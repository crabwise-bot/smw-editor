//! Per-level Layer 3 settings: the model behind the editor's
//! "Change Layer 3 Settings" dialog (Lunar Magic parity).
//!
//! Source: SMWDisX `bank_00.asm`, `CODE_009FB8` (runs during `GM11LoadLevel`).
//! The secondary header's 2-bit `Layer3` field (0-3) selects one of three
//! bytes from `Layer3TilemapSettings` for the level's `ObjectTileset`
//! (0-15). Byte 0 (setting 1) is the tide type; the game then picks the
//! stripe image via `Layer3Ptr[tilemap*3 + (setting-1)]` and uploads it with
//! `LoadStripeImage` into the 64x64 Layer 3 tilemap at VRAM word `$5000`.
//!
//! ```text
//! Layer3TilemapSettings:            ; SNES $00:9F88, 16 tilesets x 3 bytes
//!     db 1, 2, $C0                 ; tileset 0
//!     db 1, $80, $81               ; tileset 1
//!     ...                          ; (see `resolve_layer3`)
//! Layer3Ptr:                       ; SNES $05:9000, 16 tilesets x 3 pointers
//!     dl Tilemap_L3Tide, Tilemap_L3Tide, Tilemap_L3Cage   ; tileset 0
//!     ...
//! ```
//!
//! The tide bytes mean (from `CODE_009FB8`):
//! - `1`: tide that rises/falls (`Layer3YPos` oscillates `$40`/`$70`)
//! - `2`: stationary tide (`Layer3YPos` fixed `$70`)
//! - `$80`: crusher palette (`BigCrusherColors`) + scrolling Layer 3
//! - `$81`: Layer 3 follows Layer 1 at half X speed on castle/underground
//!   tilesets (3/7/10/12/13), otherwise a static background; `Layer3YPos=$C0`
//! - `$C0`: scrolling Layer 3 (`Layer3ScrollType` set), `Layer3YPos=$D0`
//!
//! The stripe images use the same `LoadStripeImage` encoding as the title
//! screen (see `title_stripe`), including RLE commands (flag bit 6) and the
//! vertical-transfer flag (bit 7); the destination-high byte's bit 7 ends the
//! image. `ClearOutLayer3` pre-fills the 64x64 map with word `$38FC`.
//!
//! This module also carries the per-level **Layer 3 GFX bypass**: Lunar
//! Magic's "Layer 3 GFX and tilemap bypass" is a closed-source ASM hack, so
//! vanilla SMW always renders Layer 3 from the level's own GFX. The editor
//! stores a per-level GFX-file override in a RATS block (`L3BP`) so the
//! *preview* (and any ROM carrying LM's bypass hack) can use it; applying it
//! in-game on a vanilla ROM is out of scope and documented as such.

use crate::snes_utils::addr::{AddrPc, AddrSnes};

/// SNES address of `Layer3TilemapSettings` (16 tilesets x 3 tide/scroll bytes).
pub const LAYER3_TILEMAP_SETTINGS_SNES: u32 = 0x00_9F88;
/// SNES address of `Layer3Ptr` (16 tilesets x 3 SNES pointers to stripe images).
pub const LAYER3_PTR_SNES: u32 = 0x05_9000;
/// VRAM word address of the 64x64 Layer 3 tilemap (`VRam_L3Tilemap`).
pub const LAYER3_TILEMAP_VRAM_WORD: u16 = 0x5000;
/// Tile word `ClearOutLayer3` pre-fills the map with (blank tile, palette 7).
pub const LAYER3_EMPTY_TILE_WORD: u16 = 0x38FC;
/// Layer 3 tilemap is 64x64 tiles (`HW_BGSC_Size_64x64` in `SetUpScreen`).
pub const LAYER3_TILEMAP_DIM: usize = 64;
/// Number of vanilla object tilesets (`ObjectTileset` 0-15).
pub const LAYER3_TILESET_COUNT: u8 = 16;

/// Named Layer 3 tilemap kinds (SMWDisX `Tilemap_L3*` labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer3TilemapKind {
    /// Water surface drawn at tilemap rows 32-63; Y position animated by the
    /// tide code (`Tilemap_L3Tide`, SNES `$05:9549`).
    Tide,
    /// Castle cage background (`Tilemap_L3Cage`, `$05:9087`).
    Cage,
    /// Crusher background (`Tilemap_L3Crusher`, `$05:9294`).
    Crusher,
    /// Castle window background (`Tilemap_L3Windows`, `$05:9AE0`).
    Windows,
    /// Rocky background (`Tilemap_L3Rocks`, `$05:A221`).
    Rocks,
    /// Cloud background (`Tilemap_L3Clouds`, `$05:95DE`).
    Clouds,
    /// Fish background (`Tilemap_L3Fish`, `$05:9A17`).
    Fish,
}

impl Layer3TilemapKind {
    /// Display name for the dialog and docs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tide => "Tide",
            Self::Cage => "Cage",
            Self::Crusher => "Crusher",
            Self::Windows => "Windows",
            Self::Rocks => "Rocks",
            Self::Clouds => "Clouds",
            Self::Fish => "Fish",
        }
    }

    /// Match a `Layer3Ptr` target against the known SMWDisX stripe labels.
    fn from_stripe_snes(stripe_snes: u32) -> Option<Self> {
        match stripe_snes {
            0x05_9549 => Some(Self::Tide),
            0x05_9087 => Some(Self::Cage),
            0x05_9294 => Some(Self::Crusher),
            0x05_9AE0 => Some(Self::Windows),
            0x05_A221 => Some(Self::Rocks),
            0x05_95DE => Some(Self::Clouds),
            0x05_9A17 => Some(Self::Fish),
            _ => None,
        }
    }
}

/// How the vanilla game treats the Layer 3 tilemap for a (tileset, setting)
/// pair, decoded from the `Layer3TilemapSettings` byte (`CODE_009FB8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer3Behavior {
    /// Setting 0: `Layer3Setting` is 0, `CODE_009FB8` returns immediately.
    /// No Layer 3 tilemap is uploaded at all.
    Disabled,
    /// Tide byte `1`: water that rises and falls (`Layer3YPos` `$40`/`$70`).
    TideRisingFalling,
    /// Tide byte `2`: stationary water (`Layer3YPos` fixed `$70`).
    TideStationary,
    /// Tide byte `$80`: `BigCrusherColors` palette + scrolling Layer 3
    /// (`Layer3ScrollType` incremented, `Layer3YPos=$D0`).
    CrusherPaletteScroll,
    /// Tide byte `$C0`: scrolling Layer 3 background, `Layer3YPos=$D0`.
    BackgroundScroll,
    /// Tide byte `$81`: on castle/underground tilesets (3/7/10/12/13) Layer 3
    /// follows Layer 1 at half X speed (`FollowLayer1XPos`); on other tilesets
    /// a static background. `Layer3YPos=$C0`.
    BackgroundFollow,
}

impl Layer3Behavior {
    /// One-line description for the "Change Layer 3 Settings" dialog.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Disabled => "No Layer 3 (vanilla uploads nothing)",
            Self::TideRisingFalling => "Tide: water rises and falls",
            Self::TideStationary => "Tide: stationary water",
            Self::CrusherPaletteScroll => "Special: crusher palette, scrolling Layer 3",
            Self::BackgroundScroll => "Scrolling Layer 3 background",
            Self::BackgroundFollow => "Background follows Layer 1 at half speed (castle/underground); static otherwise",
        }
    }

    fn from_tide_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::TideRisingFalling),
            0x02 => Some(Self::TideStationary),
            0x80 => Some(Self::CrusherPaletteScroll),
            0x81 => Some(Self::BackgroundFollow),
            0xC0 => Some(Self::BackgroundScroll),
            _ => None,
        }
    }
}

/// The fully resolved vanilla Layer 3 for one (tileset, setting) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer3Info {
    /// Object tileset (0-15) the level uses.
    pub tileset:     u8,
    /// Secondary-header Layer 3 setting (1-3; 0 means disabled).
    pub setting:     u8,
    /// Which stripe image the game uploads.
    pub kind:        Layer3TilemapKind,
    /// How the game scrolls/palettes it.
    pub behavior:    Layer3Behavior,
    /// SNES address of the stripe image (`Layer3Ptr` entry).
    pub stripe_snes: u32,
    /// Raw `Layer3TilemapSettings` byte.
    pub tide_byte:   u8,
}

fn lorom_pc(snes: u32) -> Option<usize> {
    AddrPc::try_from_lorom(AddrSnes(snes)).ok().map(|AddrPc(pc)| pc as usize)
}

fn read_u24_le(rom: &[u8], pc: usize) -> Option<u32> {
    let b = rom.get(pc..pc + 3)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], 0]))
}

/// Resolve the vanilla Layer 3 for a level's tileset and secondary-header
/// Layer 3 setting.
///
/// Returns `None` when the setting is 0 (Layer 3 disabled), the tileset or
/// setting is outside the vanilla ranges, or the ROM tables point outside the
/// ROM / at an unknown stripe (ROM hacks that repoint `Layer3Ptr`).
pub fn resolve_layer3(rom: &[u8], tileset: u8, setting: u8) -> Option<Layer3Info> {
    if setting == 0 || setting > 3 || tileset >= LAYER3_TILESET_COUNT {
        return None;
    }
    let settings_pc = lorom_pc(LAYER3_TILEMAP_SETTINGS_SNES)?;
    let tide_byte = *rom.get(settings_pc + tileset as usize * 3 + (setting as usize - 1))?;
    let ptr_pc = lorom_pc(LAYER3_PTR_SNES)?;
    let entry_pc = ptr_pc + (tileset as usize * 3 + (setting as usize - 1)) * 3;
    let stripe_snes = read_u24_le(rom, entry_pc)?;
    // The pointer must land inside the ROM so the stripe can actually be read.
    let stripe_pc = lorom_pc(stripe_snes)?;
    if rom.get(stripe_pc).is_none() {
        return None;
    }
    Some(Layer3Info {
        tileset,
        setting,
        kind: Layer3TilemapKind::from_stripe_snes(stripe_snes)?,
        behavior: Layer3Behavior::from_tide_byte(tide_byte)?,
        stripe_snes,
        tide_byte,
    })
}

/// Human-readable summary for the dialog, e.g.
/// `"3: Cage — Scrolling Layer 3 background"`.
pub fn describe_layer3_setting(rom: &[u8], tileset: u8, setting: u8) -> String {
    match resolve_layer3(rom, tileset, setting) {
        None if setting == 0 => "0: No Layer 3".to_string(),
        None => format!("{setting}: (unrecognized — ROM hack repointed the tables?)"),
        Some(info) => format!("{}: {} — {}", info.setting, info.kind.name(), info.behavior.describe()),
    }
}

// -------------------------------------------------------------------------------------------------
// Stripe-image decoding (with RLE, unlike the title-screen-only parser).
// -------------------------------------------------------------------------------------------------

/// One `LoadStripeImage` command: a run of tile words for one tilemap row
/// (or column when `vertical`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripeCommand {
    /// Destination VRAM word address (tilemap space).
    pub dest:     u16,
    /// Flag bit 7: transfer runs vertically instead of horizontally.
    pub vertical: bool,
    /// Flag bit 6: RLE — `words` holds the single repeated tile word.
    pub rle:      bool,
    /// Tile words in transfer order (length 1 for RLE).
    pub words:    Vec<u16>,
    /// Number of tiles this command paints (`words.len()`, or the RLE repeat
    /// count when `rle`).
    pub tiles:    usize,
}

/// Errors decoding a stripe image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StripeError {
    /// Truncated command header or payload.
    Truncated,
    /// A command wrote outside the 64x64 tilemap.
    OutOfBounds,
}

/// Decode a `LoadStripeImage` stripe (`SMWDisX` `CODE_00A01F`).
///
/// Header per command: destination-high, destination-low, flags, length-low.
/// Flags bit 7 = vertical transfer, bit 6 = RLE/fixed-source (one stored tile
/// word repeated for the whole transfer), low 6 bits + length-low = transfer
/// byte count minus one. A destination-high byte with bit 7 set terminates
/// the image. (Matches the hardware format; verified against every vanilla
/// `Tilemap_L3*` stripe in the real ROM.)
pub fn decode_stripe_image(bytes: &[u8]) -> Result<Vec<StripeCommand>, StripeError> {
    let mut commands = Vec::new();
    let mut i = 0;
    loop {
        let b0 = *bytes.get(i).ok_or(StripeError::Truncated)?;
        if b0 & 0x80 != 0 {
            break;
        }
        let dest_lo = *bytes.get(i + 1).ok_or(StripeError::Truncated)?;
        let flags = *bytes.get(i + 2).ok_or(StripeError::Truncated)?;
        let len_lo = *bytes.get(i + 3).ok_or(StripeError::Truncated)?;
        let dest = u16::from_be_bytes([b0, dest_lo]);
        let vertical = flags & 0x80 != 0;
        let rle = flags & 0x40 != 0;
        let transfer_bytes = ((((flags & 0x3F) as usize) << 8) | len_lo as usize) + 1;
        // RLE stores a single tile word; the transfer count says how many
        // times the DMA repeats it. Plain commands store every word.
        let payload_bytes = if rle { 2 } else { transfer_bytes };
        if payload_bytes % 2 != 0 {
            return Err(StripeError::Truncated);
        }
        let payload = bytes.get(i + 4..i + 4 + payload_bytes).ok_or(StripeError::Truncated)?;
        let mut words = Vec::with_capacity(payload_bytes / 2);
        for pair in payload.chunks_exact(2) {
            // Tile words are little-endian in the ROM (`dw $397D` assembles
            // to `7D 39`); the destination word is big-endian.
            words.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        let tiles = transfer_bytes / 2;
        // Bounds-check the transfer against the 64x64 tilemap at $5000.
        let base = dest.wrapping_sub(LAYER3_TILEMAP_VRAM_WORD) as usize;
        let end = if vertical { base + (tiles - 1) * 64 } else { base + (tiles - 1) };
        if base >= 64 * 64 || end >= 64 * 64 {
            return Err(StripeError::OutOfBounds);
        }
        commands.push(StripeCommand { dest, vertical, rle, words, tiles });
        i += 4 + payload_bytes;
    }
    Ok(commands)
}

/// Apply decoded stripe commands to a 64x64 tile-word grid, starting from the
/// `$38FC` fill `ClearOutLayer3` leaves behind.
pub fn stripe_to_tilemap(commands: &[StripeCommand]) -> [[u16; LAYER3_TILEMAP_DIM]; LAYER3_TILEMAP_DIM] {
    let mut grid = [[LAYER3_EMPTY_TILE_WORD; LAYER3_TILEMAP_DIM]; LAYER3_TILEMAP_DIM];
    for cmd in commands {
        let mut addr = cmd.dest.wrapping_sub(LAYER3_TILEMAP_VRAM_WORD) as usize;
        let step = if cmd.vertical { 64 } else { 1 };
        if cmd.rle {
            let word = cmd.words[0];
            for _ in 0..cmd.tiles {
                let (y, x) = (addr / 64, addr % 64);
                grid[y][x] = word;
                addr += step;
            }
        } else {
            for &word in &cmd.words {
                let (y, x) = (addr / 64, addr % 64);
                grid[y][x] = word;
                addr += step;
            }
        }
    }
    grid
}

/// Decode the stripe image at a SNES address and apply it to a 64x64 grid.
///
/// Returns `None` when the address is outside the ROM or the stripe is
/// malformed.
pub fn layer3_tilemap(rom: &[u8], stripe_snes: u32) -> Option<[[u16; 64]; 64]> {
    let pc = lorom_pc(stripe_snes)?;
    // Bound the slice: stripes end at the first dest-high byte with bit 7
    // set; cap the scan at 8 KiB (largest vanilla stripe is ~1.2 KiB).
    let bytes = rom.get(pc..(pc + 8192).min(rom.len()))?;
    let commands = decode_stripe_image(bytes).ok()?;
    Some(stripe_to_tilemap(&commands))
}

// -------------------------------------------------------------------------------------------------
// Per-level Layer 3 GFX bypass (RATS block).
// -------------------------------------------------------------------------------------------------

/// Magic tag identifying our RATS data block.
pub const LAYER3_BYPASS_TAG: &[u8; 4] = b"L3BP";
/// Payload version byte.
pub const LAYER3_BYPASS_VERSION: u8 = 1;
/// `gfx_file` value meaning "no override — use the level's own GFX".
pub const LAYER3_BYPASS_NONE: u8 = 0xFF;

/// Per-level Layer 3 GFX-file overrides.
///
/// Lunar Magic's "Layer 3 GFX and tilemap bypass" is a closed-source ASM hack;
/// vanilla SMW always renders Layer 3 from the level's own GFX files. The
/// editor stores overrides here so the WYSIWYG preview (and ROMs carrying
/// LM's hack) can honor them. Each entry is `(level_number, gfx_file)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer3GfxBypass {
    entries: Vec<(u16, u8)>,
}

impl Layer3GfxBypass {
    /// Load overrides from the ROM's `L3BP` RATS block, if present.
    pub fn load(rom: &[u8]) -> Self {
        let mut out = Self::default();
        let Some(payload) = find_bypass_payload(rom) else { return out };
        if payload.len() < 6 || &payload[0..4] != LAYER3_BYPASS_TAG || payload[4] != LAYER3_BYPASS_VERSION {
            return out;
        }
        let count = payload[5] as usize;
        for e in payload.get(6..6 + count * 3).unwrap_or(&[]).chunks_exact(3) {
            let level = u16::from_le_bytes([e[0], e[1]]);
            if e[2] != LAYER3_BYPASS_NONE {
                out.entries.push((level, e[2]));
            }
        }
        out
    }

    /// GFX-file override for a level, if one is stored.
    pub fn get(&self, level: u16) -> Option<u8> {
        self.entries.iter().find(|(l, _)| *l == level).map(|(_, g)| *g)
    }

    /// Set (or clear with `None`) the override for a level.
    pub fn set(&mut self, level: u16, gfx_file: Option<u8>) {
        self.entries.retain(|(l, _)| *l != level);
        if let Some(g) = gfx_file {
            self.entries.push((level, g));
            self.entries.sort_unstable();
        }
    }

    /// Number of stored overrides.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Serialize to a RATS data block (including the `RATS` header).
    pub fn to_rats_block(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(6 + self.entries.len() * 3);
        payload.extend_from_slice(LAYER3_BYPASS_TAG);
        payload.push(LAYER3_BYPASS_VERSION);
        payload.push(self.entries.len().min(255) as u8);
        for (level, gfx) in self.entries.iter().take(255) {
            payload.extend_from_slice(&level.to_le_bytes());
            payload.push(*gfx);
        }
        let size = payload.len() as u16;
        let mut block = Vec::with_capacity(8 + payload.len());
        block.extend_from_slice(b"RATS");
        block.extend_from_slice(&size.to_le_bytes());
        block.extend_from_slice(&(!size).to_le_bytes());
        block.extend_from_slice(&payload);
        block
    }

    /// Write the bypass table to the ROM: erase stale `L3BP` blocks, then
    /// append a fresh RATS block at free space (if non-empty).
    ///
    /// `header_offset` is 0x200 for SMC-headered ROMs, 0 otherwise.
    /// Returns `false` when the table is non-empty but no free space was found.
    pub fn save_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> bool {
        erase_bypass_blocks(rom_bytes);
        if self.entries.is_empty() {
            return true;
        }
        let block = self.to_rats_block();
        let Some(pc) = crate::freespace::find_free_space(rom_bytes, block.len(), 0x008000, header_offset) else {
            return false;
        };
        rom_bytes[pc..pc + block.len()].copy_from_slice(&block);
        true
    }
}

/// Locate the `L3BP` RATS payload in the ROM, validating the RATS header.
fn find_bypass_payload(rom: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while i + 8 <= rom.len() {
        if &rom[i..i + 4] == b"RATS" {
            let size = u16::from_le_bytes([rom[i + 4], rom[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom[i + 6], rom[i + 7]]);
            if inv == !size as u16 && rom.len() >= i + 8 + size {
                let payload = &rom[i + 8..i + 8 + size];
                if payload.len() >= 4 && &payload[0..4] == LAYER3_BYPASS_TAG {
                    return Some(payload);
                }
                i += 8 + size;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Erase every `L3BP` RATS block in the ROM (fills with 0xFF, the erased
/// state), so a fresh block can be written without leaving stale copies.
pub fn erase_bypass_blocks(rom: &mut [u8]) {
    let mut i = 0;
    while i + 8 <= rom.len() {
        if &rom[i..i + 4] == b"RATS" {
            let size = u16::from_le_bytes([rom[i + 4], rom[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom[i + 6], rom[i + 7]]);
            if inv == !size as u16 && rom.len() >= i + 8 + size {
                if rom[i + 8..i + 12] == *LAYER3_BYPASS_TAG {
                    rom[i..i + 8 + size].fill(0xFF);
                }
                i += 8 + size;
                continue;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Path to the real ROM for ignored hardware-validation tests.
    fn rom_path() -> Option<std::path::PathBuf> {
        std::env::var("ROM_PATH").ok().map(std::path::PathBuf::from)
    }

    #[test]
    fn tide_byte_decoding_covers_all_vanilla_values() {
        assert_eq!(Layer3Behavior::from_tide_byte(0x01), Some(Layer3Behavior::TideRisingFalling));
        assert_eq!(Layer3Behavior::from_tide_byte(0x02), Some(Layer3Behavior::TideStationary));
        assert_eq!(Layer3Behavior::from_tide_byte(0x80), Some(Layer3Behavior::CrusherPaletteScroll));
        assert_eq!(Layer3Behavior::from_tide_byte(0x81), Some(Layer3Behavior::BackgroundFollow));
        assert_eq!(Layer3Behavior::from_tide_byte(0xC0), Some(Layer3Behavior::BackgroundScroll));
        assert_eq!(Layer3Behavior::from_tide_byte(0x00), None);
    }

    #[test]
    fn stripe_kind_matches_all_vanilla_pointers() {
        for snes in [0x05_9549u32, 0x05_9087, 0x05_9294, 0x05_9AE0, 0x05_A221, 0x05_95DE, 0x05_9A17] {
            assert!(Layer3TilemapKind::from_stripe_snes(snes).is_some(), "{snes:#X}");
        }
        assert_eq!(Layer3TilemapKind::from_stripe_snes(0x05_0000), None);
    }

    #[test]
    fn rle_stripe_decodes_single_word_repeated() {
        // dest $5800, flags RLE (bit 6), 4-byte transfer of one word $397D
        // (i.e. 2 tiles; the word is little-endian in the ROM), then terminator.
        let bytes = [0x58, 0x00, 0x40, 0x03, 0x7D, 0x39, 0x80];
        let cmds = decode_stripe_image(&bytes).unwrap();
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].rle);
        assert_eq!(cmds[0].words, vec![0x397D]);
        assert_eq!(cmds[0].tiles, 2);
        let grid = stripe_to_tilemap(&cmds);
        assert_eq!(grid[32][0], 0x397D);
        assert_eq!(grid[32][1], 0x397D);
        assert_eq!(grid[0][0], LAYER3_EMPTY_TILE_WORD);
    }

    #[test]
    fn bypass_round_trips_through_rats() {
        let mut b = Layer3GfxBypass::default();
        assert_eq!(b.get(0x24), None);
        b.set(0x24, Some(0x2A));
        b.set(0x105, Some(0x0B));
        assert_eq!(b.get(0x24), Some(0x2A));
        let block = b.to_rats_block();
        assert_eq!(&block[0..4], b"RATS");
        let size = u16::from_le_bytes([block[4], block[5]]) as usize;
        assert_eq!(block.len(), 8 + size);
        assert_eq!(&block[8..12], LAYER3_BYPASS_TAG);

        let mut rom = vec![0xFFu8; 0x8000];
        rom[0x1000..0x1000 + block.len()].copy_from_slice(&block);
        let loaded = Layer3GfxBypass::load(&rom);
        assert_eq!(loaded.get(0x24), Some(0x2A));
        assert_eq!(loaded.get(0x105), Some(0x0B));
        assert_eq!(loaded.get(0x25), None);

        b.set(0x24, None);
        assert_eq!(b.get(0x24), None);
        assert_eq!(b.len(), 1);
    }

    #[test]
    #[ignore]
    fn real_rom_tables_resolve_every_vanilla_combination() {
        let Some(path) = rom_path() else { return };
        let rom = std::fs::read(path).unwrap();
        // Every (tileset, setting) with setting 1-3 resolves on vanilla.
        for tileset in 0..15 {
            for setting in 1..=3 {
                let info = resolve_layer3(&rom, tileset, setting)
                    .unwrap_or_else(|| panic!("tileset {tileset} setting {setting}"));
                assert!(lorom_pc(info.stripe_snes).is_some());
            }
        }
        // Setting 0 disables; tileset 15 (unused) has garbage pointers.
        assert_eq!(resolve_layer3(&rom, 0, 0), None);
        assert_eq!(resolve_layer3(&rom, 15, 1), None);
        assert_eq!(resolve_layer3(&rom, 0, 4), None);
        // Spot-check the well-known entries.
        let tide = resolve_layer3(&rom, 0, 1).unwrap();
        assert_eq!(tide.kind, Layer3TilemapKind::Tide);
        assert_eq!(tide.behavior, Layer3Behavior::TideRisingFalling);
        assert_eq!(tide.stripe_snes, 0x05_9549);
        let windows = resolve_layer3(&rom, 1, 3).unwrap();
        assert_eq!(windows.kind, Layer3TilemapKind::Windows);
        assert_eq!(windows.behavior, Layer3Behavior::BackgroundFollow);
    }

    #[test]
    #[ignore]
    fn real_rom_tide_stripe_decodes_with_rle() {
        let Some(path) = rom_path() else { return };
        let rom = std::fs::read(path).unwrap();
        let info = resolve_layer3(&rom, 0, 1).unwrap();
        let pc = lorom_pc(info.stripe_snes).unwrap();
        let cmds = decode_stripe_image(&rom[pc..pc + 4096]).unwrap();
        // Tide (`Tilemap_L3Tide` at $05:9549): a 32-tile plain row at $5800,
        // an RLE fill from $5820, then the same pair at $5C00/$5C20.
        assert_eq!(cmds.len(), 4);
        assert!(!cmds[0].rle && !cmds[2].rle);
        assert!(cmds[1].rle && cmds[3].rle);
        assert_eq!(cmds[0].dest, 0x5800);
        assert_eq!(cmds[0].words[0], 0x397D);
        assert_eq!(cmds[1].dest, 0x5820);
        assert_eq!(cmds[1].words, vec![0x398E]);
        assert_eq!(cmds[2].dest, 0x5C00);
        assert_eq!(cmds[3].dest, 0x5C20);
        let grid = stripe_to_tilemap(&cmds);
        // Water tiles land at rows 32-63; the RLE fill repeats $398E.
        assert_eq!(grid[32][0], 0x397D);
        assert_eq!(grid[32][32], 0x398E);
        // Untouched rows keep the ClearOutLayer3 fill.
        assert_eq!(grid[0][0], LAYER3_EMPTY_TILE_WORD);
    }

    #[test]
    #[ignore]
    fn real_rom_all_stripes_decode_cleanly() {
        let Some(path) = rom_path() else { return };
        let rom = std::fs::read(path).unwrap();
        let mut seen = std::collections::HashSet::new();
        for tileset in 0..15 {
            for setting in 1..=3 {
                let info = resolve_layer3(&rom, tileset, setting).unwrap();
                if !seen.insert(info.stripe_snes) {
                    continue;
                }
                let grid = layer3_tilemap(&rom, info.stripe_snes)
                    .unwrap_or_else(|| panic!("stripe ${:06X} failed", info.stripe_snes));
                // Every stripe paints at least one non-blank tile.
                assert!(
                    grid.iter().flatten().any(|&w| w != LAYER3_EMPTY_TILE_WORD),
                    "stripe ${:06X} painted nothing",
                    info.stripe_snes
                );
            }
        }
        // 7 distinct vanilla stripes.
        assert_eq!(seen.len(), 7);
    }
}
