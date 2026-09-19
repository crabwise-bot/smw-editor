//! WYSIWYG codec for the title screen's Layer 3 tilemap.
//!
//! The title screen logo and menu ("SUPER MARIO WORLD", "1 PLAYER GAME", …)
//! are a 64×64 Layer 3 tilemap at VRAM `$5000` (`VRam_L3Tilemap` in SMWDisX;
//! `BG3SC` is `$50 | Size_64x64` in `SetUpScreen`), drawn by the fixed-slot
//! stripe image at [`crate::title_credits::TITLE_SCREEN_STRIPE_SNES`]
//! (`0x05B375..0x05B7C9` on the U ROM). This module parses that stripe image
//! into an editable 64×64 grid of SNES tile words and re-encodes an edited
//! grid back to stripe bytes, using the exact `LoadStripeImage` semantics
//! from SMWDisX `bank_00.asm`:
//!
//! - each command is `[VRAM-dest-hi][VRAM-dest-lo][flags][len-lo][payload …]`;
//! - flags bit 7: 0 = horizontal (1-word stride), 1 = vertical (32-word
//!   stride); bit 6: RLE; bits 5–0 plus `len-lo` form the 14-bit payload byte
//!   count minus 1;
//! - payload tile words are little-endian;
//! - a first byte with bit 7 set terminates the image (the editor always
//!   writes the `$FF` terminator).
//!
//! Note the vertical stride (32 words) against the 64×64 tilemap's four
//! 32×32 screens: one vertical step moves to the next visual row *within the
//! same 32×32 block* (see [`tilemap_64x64_word_offset`]). The parser
//! reproduces this exactly; commands are applied in order so overlapping
//! writes compose like the hardware DMA does.
//!
//! RLE commands are decoded (expanded). The encoder emits raw horizontal runs
//! and never emits RLE; callers that must preserve RLE (e.g. the player-select
//! stripe's blank clears) keep those commands verbatim.
//!
//! ## Encoding
//!
//! The grid re-encodes as canonical non-overlapping horizontal runs of
//! non-blank cells over a [`TITLE_TILEMAP_BLANK`] (`$38FC`/`!EmptyTile`)
//! background — the value `ClearOutLayer3` fills the Layer 3 tilemap with in
//! `GM04PrepTitleScreen` before the stripe uploads. The encoded stripe is
//! DMA-equivalent to the displayed grid: applying it over a `$38FC`
//! background reproduces the grid exactly. Encoded size is checked against
//! [`crate::title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE`]; over-budget grids
//! are refused, never silently truncated.
//!
//! Region scope: U-ROM fixed-slot addresses. J/E ROMs place the title stripe
//! elsewhere (see `differences.txt` in SMWDisX); only the U layout is
//! modeled.

use crate::title_credits::{PLAYER_SELECT_STRIPE_MAX_SIZE, TITLE_SCREEN_STRIPE_MAX_SIZE};

/// Width/height of the title Layer 3 tilemap in tiles (`Size_64x64`).
pub const TITLE_TILEMAP_WIDTH: usize = 64;
pub const TITLE_TILEMAP_HEIGHT: usize = 64;
/// VRAM word address of the Layer 3 tilemap (`VRam_L3Tilemap`).
pub const TITLE_TILEMAP_VRAM_BASE: u16 = 0x5000;
/// Tile word `ClearOutLayer3` fills the Layer 3 tilemap with before the
/// title stripe uploads (`!EmptyTile` in SMWDisX `constants.asm`). Cells
/// holding this value are "blank" and skipped by the encoder.
pub const TITLE_TILEMAP_BLANK: u16 = 0x38FC;

/// PPU word offset (from [`TITLE_TILEMAP_VRAM_BASE`]) of tile `(x, y)` in a
/// 64×64 SNES tilemap.
///
/// A 64×64 tilemap is four contiguous 32×32 screens: block 0 top-left, block
/// 1 top-right, block 2 bottom-left, block 3 bottom-right, each 0x400 words.
/// This is the mapping the PPU uses to display the tilemap; a linear
/// `y * 64 + x` address is wrong and scrambles/misplaces rows.
pub fn tilemap_64x64_word_offset(x: usize, y: usize) -> usize {
    debug_assert!(x < 64 && y < 64);
    let block = (y / 32) * 2 + (x / 32);
    block * 0x400 + (y % 32) * 32 + (x % 32)
}

/// Inverse of [`tilemap_64x64_word_offset`]: the `(x, y)` tile coordinates
/// the PPU displays for a word offset from [`TITLE_TILEMAP_VRAM_BASE`].
pub fn tilemap_64x64_xy(word_offset: usize) -> (usize, usize) {
    debug_assert!(word_offset < 0x1000);
    let block = word_offset / 0x400;
    let inside = word_offset % 0x400;
    ((block % 2) * 32 + inside % 32, (block / 2) * 32 + inside / 32)
}

/// One parsed stripe-image command (see `LoadStripeImage` in SMWDisX
/// `bank_00.asm`).
#[derive(Debug, Clone)]
pub struct TitleStripeCommand {
    /// VRAM word destination.
    pub vram_dest: u16,
    /// True = vertical (32-word stride), false = horizontal (1-word stride).
    pub vertical:  bool,
    /// True = RLE: `tiles` holds the single repeated tile word.
    pub rle:       bool,
    /// Raw payload byte count from the flags (`nbytes`). For RLE this may be
    /// odd (the vanilla player-select stripe uses 29); the repeat count is
    /// `nbytes / 2`.
    pub nbytes:    usize,
    /// Payload tile words, little-endian in the stripe. For RLE this is the
    /// single repeated word.
    pub tiles:     Vec<u16>,
}

impl TitleStripeCommand {
    /// Number of tile words this command writes (RLE expanded).
    pub fn tile_count(&self) -> usize {
        if self.rle {
            self.nbytes / 2
        } else {
            self.tiles.len()
        }
    }

    /// Tile words this command writes, in order (RLE expanded).
    pub fn expanded_tiles(&self) -> Vec<u16> {
        if self.rle {
            vec![self.tiles[0]; self.tile_count()]
        } else {
            self.tiles.clone()
        }
    }
}

/// Parse raw stripe-image bytes into commands.
///
/// Stops at the first byte with bit 7 set (the `$FF` terminator); any bytes
/// after it are ignored. Errors on truncated headers/payloads and odd payload
/// lengths. RLE commands are decoded (the repeated tile is expanded).
pub fn parse_title_stripe(bytes: &[u8]) -> anyhow::Result<Vec<TitleStripeCommand>> {
    let mut commands = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] & 0x80 != 0 {
            break;
        }
        if i + 4 > bytes.len() {
            anyhow::bail!("truncated title stripe header at offset {i:#X}");
        }
        let vram_dest = u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        let flags = bytes[i + 2];
        let rle = flags & 0x40 != 0;
        let nbytes = (((flags & 0x3F) as usize) << 8 | bytes[i + 3] as usize) + 1;
        // RLE payload is a single tile word (2 bytes); nbytes is the total
        // repeated byte count (may be odd in vanilla data). Raw payload is
        // nbytes bytes and must be even.
        let payload_len = if rle { 2 } else { nbytes };
        if !rle && nbytes % 2 != 0 {
            anyhow::bail!("odd title stripe payload length {nbytes} at offset {i:#X}");
        }
        let payload_end = i + 4 + payload_len;
        if payload_end > bytes.len() {
            anyhow::bail!("truncated title stripe payload at offset {i:#X}");
        }
        let tiles: Vec<u16> =
            bytes[i + 4..payload_end].chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect();
        if rle && tiles.len() != 1 {
            anyhow::bail!("RLE title stripe command with {} tiles at offset {i:#X}", tiles.len());
        }
        commands.push(TitleStripeCommand { vram_dest, vertical: flags & 0x80 != 0, rle, nbytes, tiles });
        i = payload_end;
    }
    Ok(commands)
}

/// Serialize commands back to stripe bytes, terminated with `$FF`.
/// RLE commands are written as RLE (single tile + original byte count).
pub fn serialize_title_stripe(commands: &[TitleStripeCommand]) -> Vec<u8> {
    let mut out = Vec::new();
    for cmd in commands {
        out.extend_from_slice(&cmd.vram_dest.to_be_bytes());
        let nbytes = if cmd.rle { cmd.nbytes } else { cmd.tiles.len() * 2 };
        debug_assert!(nbytes >= 1 && nbytes <= 0x4000);
        let mut flags = ((nbytes - 1) >> 8) as u8 & 0x3F;
        if cmd.vertical {
            flags |= 0x80;
        }
        if cmd.rle {
            flags |= 0x40;
        }
        out.push(flags);
        out.push(((nbytes - 1) & 0xFF) as u8);
        if cmd.rle {
            out.extend_from_slice(&cmd.tiles[0].to_le_bytes());
        } else {
            for &tile in &cmd.tiles {
                out.extend_from_slice(&tile.to_le_bytes());
            }
        }
    }
    out.push(0xFF);
    out
}

/// 64×64 grid of SNES tile words (row-major), exactly as the PPU displays
/// the title Layer 3 tilemap after the stripe uploads.
#[derive(Debug, Clone)]
pub struct TitleTileGrid {
    pub cells: [[u16; TITLE_TILEMAP_WIDTH]; TITLE_TILEMAP_HEIGHT],
}

impl TitleTileGrid {
    /// Blank grid (every cell [`TITLE_TILEMAP_BLANK`]).
    pub fn blank() -> Self {
        Self { cells: [[TITLE_TILEMAP_BLANK; TITLE_TILEMAP_WIDTH]; TITLE_TILEMAP_HEIGHT] }
    }

    /// Apply stripe commands to a blank grid with the exact `LoadStripeImage`
    /// semantics: the VRAM word address advances by 1 word (horizontal) or 32
    /// words (vertical) per tile. RLE commands repeat their tile. Later
    /// commands overwrite earlier words, so overlapping commands compose
    /// exactly like the hardware DMA does. VRAM word offsets map to grid
    /// coordinates with the PPU's 64×64 screen-block layout
    /// ([`tilemap_64x64_xy`]), so the grid matches what the PPU displays.
    pub fn from_commands(commands: &[TitleStripeCommand]) -> Self {
        let mut cells = [[TITLE_TILEMAP_BLANK; TITLE_TILEMAP_WIDTH]; TITLE_TILEMAP_HEIGHT];
        let base = TITLE_TILEMAP_VRAM_BASE as usize;
        // Word offset of the last tilemap word from the base.
        let max_word = TITLE_TILEMAP_WIDTH * TITLE_TILEMAP_HEIGHT;
        for cmd in commands {
            let mut dest = cmd.vram_dest as usize;
            let stride = if cmd.vertical { 32 } else { 1 };
            for &tile in &cmd.expanded_tiles() {
                if dest >= base {
                    let wo = dest - base;
                    if wo < max_word {
                        let (x, y) = tilemap_64x64_xy(wo);
                        cells[y][x] = tile;
                    }
                }
                dest += stride;
            }
        }
        Self { cells }
    }

    /// Parse raw stripe bytes and build the displayed grid.
    pub fn from_stripe(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(Self::from_commands(&parse_title_stripe(bytes)?))
    }

    /// Encode the grid as canonical horizontal runs of non-blank cells.
    ///
    /// The result is DMA-equivalent to the grid: applying it over a
    /// [`TITLE_TILEMAP_BLANK`] background reproduces every cell exactly.
    /// Runs never cross a 32-column screen-block boundary (a run from x=31
    /// to x=32 is not contiguous in VRAM). Errors if the encoding would
    /// exceed [`crate::title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE`] —
    /// over-budget edits are refused, never silently truncated.
    pub fn to_stripe_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut commands = Vec::new();
        for (y, row) in self.cells.iter().enumerate() {
            // Two 32-column screen blocks per row; a DMA run cannot span the
            // block boundary.
            for block_x in 0..2 {
                let mut x = block_x * 32;
                let end = x + 32;
                while x < end {
                    if row[x] == TITLE_TILEMAP_BLANK {
                        x += 1;
                        continue;
                    }
                    let x0 = x;
                    while x < end && row[x] != TITLE_TILEMAP_BLANK {
                        x += 1;
                    }
                    commands.push(TitleStripeCommand {
                        vram_dest: TITLE_TILEMAP_VRAM_BASE + tilemap_64x64_word_offset(x0, y) as u16,
                        vertical:  false,
                        rle:       false,
                        nbytes:    (x - x0) * 2,
                        tiles:     row[x0..x].to_vec(),
                    });
                }
            }
        }
        let bytes = serialize_title_stripe(&commands);
        if bytes.len() > TITLE_SCREEN_STRIPE_MAX_SIZE {
            anyhow::bail!(
                "Encoded title stripe is {} bytes, but the vanilla fixed slot is only {TITLE_SCREEN_STRIPE_MAX_SIZE} bytes; \
                 erase some tiles to fit",
                bytes.len()
            );
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built stripe: one horizontal command writing 3 tiles at (0,0),
    /// one vertical command writing 2 tiles at dest $500A (word offset 10 →
    /// (10,0); 32-word stride → word offset 42 → (10,1), the next row in the
    /// same 32×32 screen block).
    fn sample_stripe() -> Vec<u8> {
        vec![
            0x50, 0x00, 0x00, 0x05, // dest $5000, horizontal, 6 bytes
            0x58, 0x2C, 0x59, 0x2C, 0x38, 0x2C, // $2C58 $2C59 $2C38
            0x50, 0x0A, 0x80, 0x03, // dest $500A, vertical, 4 bytes
            0x98, 0x3C, 0xA9, 0x3C, // $3C98 $3CA9
            0xFF,
        ]
    }

    #[test]
    fn parse_sample_commands() {
        let cmds = parse_title_stripe(&sample_stripe()).unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].vram_dest, 0x5000);
        assert!(!cmds[0].vertical);
        assert_eq!(cmds[0].tiles, vec![0x2C58, 0x2C59, 0x2C38]);
        assert_eq!(cmds[1].vram_dest, 0x500A);
        assert!(cmds[1].vertical);
        assert_eq!(cmds[1].tiles, vec![0x3C98, 0x3CA9]);
    }

    #[test]
    fn serialize_round_trips_parse() {
        let bytes = sample_stripe();
        let cmds = parse_title_stripe(&bytes).unwrap();
        assert_eq!(serialize_title_stripe(&cmds), bytes);
    }

    #[test]
    fn parse_decodes_rle() {
        // RLE command: 4-byte header + 2-byte single tile payload; nbytes
        // (6) is the total repeated count (3 tiles).
        let b = vec![
            0x50, 0x00, 0x40, 0x05, // dest $5000, RLE, 6 bytes total
            0x58, 0x2C, // $2C58 repeated 3x
            0xFF,
        ];
        let cmds = parse_title_stripe(&b).unwrap();
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].rle);
        assert_eq!(cmds[0].tile_count(), 3);
        assert_eq!(cmds[0].tiles, vec![0x2C58]);
        assert_eq!(cmds[0].expanded_tiles(), vec![0x2C58, 0x2C58, 0x2C58]);
        // RLE round-trips through the serializer.
        assert_eq!(serialize_title_stripe(&cmds), b);
        // Grid applies the expanded tiles.
        let grid = TitleTileGrid::from_commands(&cmds);
        assert_eq!(grid.cells[0][0], 0x2C58);
        assert_eq!(grid.cells[0][1], 0x2C58);
        assert_eq!(grid.cells[0][2], 0x2C58);
    }

    #[test]
    fn parse_stops_at_terminator() {
        let mut b = sample_stripe();
        b.push(0x00); // trailing garbage after FF is ignored
        let cmds = parse_title_stripe(&b).unwrap();
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn grid_applies_commands_in_order() {
        let grid = TitleTileGrid::from_stripe(&sample_stripe()).unwrap();
        assert_eq!(grid.cells[0][0], 0x2C58);
        assert_eq!(grid.cells[0][1], 0x2C59);
        assert_eq!(grid.cells[0][2], 0x2C38);
        // vertical command: (10,0) and (10,1) — 32-word stride steps to the
        // next row within the same 32x32 screen block
        assert_eq!(grid.cells[0][10], 0x3C98);
        assert_eq!(grid.cells[1][10], 0x3CA9);
        // untouched cells are blank
        assert_eq!(grid.cells[0][3], TITLE_TILEMAP_BLANK);
        assert_eq!(grid.cells[0][42], TITLE_TILEMAP_BLANK);
    }

    #[test]
    fn tilemap_64x64_block_mapping() {
        // Screen-block layout: (x,y) -> block*0x400 + (y%32)*32 + (x%32).
        assert_eq!(tilemap_64x64_word_offset(0, 0), 0x000);
        assert_eq!(tilemap_64x64_word_offset(31, 0), 0x01F);
        assert_eq!(tilemap_64x64_word_offset(32, 0), 0x400);
        assert_eq!(tilemap_64x64_word_offset(63, 31), 0x7FF);
        assert_eq!(tilemap_64x64_word_offset(0, 32), 0x800);
        assert_eq!(tilemap_64x64_word_offset(32, 32), 0xC00);
        assert_eq!(tilemap_64x64_word_offset(63, 63), 0xFFF);
        // Inverse mapping round-trips.
        for y in [0, 1, 31, 32, 33, 63] {
            for x in [0, 1, 31, 32, 33, 63] {
                let wo = tilemap_64x64_word_offset(x, y);
                assert_eq!(tilemap_64x64_xy(wo), (x, y), "round-trip ({x},{y})");
            }
        }
        // $500A + 32 words steps (10,0) -> (10,1), not (42,0).
        assert_eq!(tilemap_64x64_xy(10), (10, 0));
        assert_eq!(tilemap_64x64_xy(42), (10, 1));
        // Crossing $53FF/$5400 moves between left/right screen blocks.
        assert_eq!(tilemap_64x64_xy(0x3FF), (31, 31));
        assert_eq!(tilemap_64x64_xy(0x400), (32, 0));
    }

    #[test]
    fn later_commands_overwrite_earlier_words() {
        // Two horizontal commands writing the same cell; the second wins.
        let b = vec![
            0x50, 0x00, 0x00, 0x01, 0x11, 0x11, // $1111 at (0,0)
            0x50, 0x00, 0x00, 0x01, 0x22, 0x22, // $2222 at (0,0)
            0xFF,
        ];
        let grid = TitleTileGrid::from_stripe(&b).unwrap();
        assert_eq!(grid.cells[0][0], 0x2222);
    }

    #[test]
    fn encode_is_dma_equivalent_to_grid() {
        let grid = TitleTileGrid::from_stripe(&sample_stripe()).unwrap();
        let bytes = grid.to_stripe_bytes().unwrap();
        let grid2 = TitleTileGrid::from_stripe(&bytes).unwrap();
        assert_eq!(grid.cells, grid2.cells);
    }

    #[test]
    fn encode_refuses_over_budget() {
        // Checkerboard of isolated single tiles: far more than the 1108-byte
        // budget allows (each 1-tile command costs 6 bytes).
        let mut grid = TitleTileGrid::blank();
        for (y, row) in grid.cells.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                if (x + y) % 2 == 0 {
                    *cell = 0x2C00 | ((y * 64 + x) as u16 & 0x3FF);
                }
            }
        }
        assert!(grid.to_stripe_bytes().is_err());
    }

    #[test]
    fn blank_grid_encodes_to_terminator_only() {
        let grid = TitleTileGrid::blank();
        assert_eq!(grid.to_stripe_bytes().unwrap(), vec![0xFF]);
    }
}

#[cfg(test)]
mod real_rom_tests {
    use super::*;
    use crate::{snes_utils::addr::AddrPc, title_credits::TITLE_SCREEN_STRIPE_SNES, SmwRom};

    fn real_stripe_bytes(rom_path: &str) -> Vec<u8> {
        let raw = std::fs::read(rom_path).expect("read ROM");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
        let start = AddrPc::try_from_lorom(TITLE_SCREEN_STRIPE_SNES).unwrap().as_index();
        let slot = &rom_bytes[start..start + TITLE_SCREEN_STRIPE_MAX_SIZE];
        let end = slot.iter().position(|&b| b == 0xFF).map(|p| p + 1).unwrap_or(slot.len());
        slot[..end].to_vec()
    }

    /// Parses the real title stripe and checks the composite grid decodes to
    /// the known vanilla layout: row 0 holds the 64-wide top of the "SUPER
    /// MARIO WORLD" logo (palette 3), and the re-encoded stripe is
    /// DMA-equivalent to the parsed grid while fitting the fixed-slot budget.
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib --
    /// --ignored real_rom_title_stripe`.
    #[test]
    #[ignore]
    fn real_rom_title_stripe() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let bytes = real_stripe_bytes(&rom_path);
        println!("real title stripe: {} bytes", bytes.len());

        let cmds = parse_title_stripe(&bytes).expect("parse real title stripe");
        println!("parsed {} commands", cmds.len());
        assert_eq!(cmds.len(), 17);
        assert!(!cmds.iter().any(|c| c.vertical && c.tiles.is_empty()));

        // Parser → serializer round-trips byte-identically (codec fidelity).
        assert_eq!(serialize_title_stripe(&cmds), bytes);

        let grid = TitleTileGrid::from_commands(&cmds);
        // Row 0 is the top of the "SUPER MARIO WORLD" logo: palette-3 tiles
        // across x 0-31 (the left 32x32 screen block; x 32-63 is blank).
        for x in 0..32 {
            let w = grid.cells[0][x];
            assert_ne!(w, TITLE_TILEMAP_BLANK, "row 0 col {x} should be logo");
            assert_eq!((w >> 10) & 7, 3, "row 0 col {x}: expected palette 3, got {w:#06X}");
        }
        for x in 32..64 {
            assert_eq!(grid.cells[0][x], TITLE_TILEMAP_BLANK, "row 0 col {x} should be blank");
        }
        // The logo's vertical strips live in columns 0-1 (32-word stride
        // steps to the next row within the same 32x32 block).
        assert_ne!(grid.cells[1][0], TITLE_TILEMAP_BLANK);
        assert_ne!(grid.cells[1][1], TITLE_TILEMAP_BLANK);

        // Normalized re-encode is DMA-equivalent and fits the budget.
        let reencoded = grid.to_stripe_bytes().expect("re-encode fits budget");
        println!("normalized re-encode: {} bytes / {TITLE_SCREEN_STRIPE_MAX_SIZE} budget", reencoded.len());
        let grid2 = TitleTileGrid::from_stripe(&reencoded).unwrap();
        assert_eq!(grid.cells, grid2.cells);
    }

    /// Cross-checks the grid codec against the ROM as loaded through
    /// `SmwRom` (the same path the editor uses).
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib --
    /// --ignored real_rom_title_stripe_via_smwrom`.
    #[test]
    #[ignore]
    fn real_rom_title_stripe_via_smwrom() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(&rom_path).expect("parse ROM");
        let grid =
            TitleTileGrid::from_stripe(&rom.title_credits.title_screen_stripe).expect("parse title stripe via SmwRom");
        let nonzero = grid.cells.iter().flatten().filter(|&&w| w != TITLE_TILEMAP_BLANK).count();
        println!("non-blank cells: {nonzero}");
        assert!(nonzero > 300, "expected a densely-drawn logo, found {nonzero} cells");
    }
}

/// Player-select stripe (`PlayerSelectStripe`): the "1 PLAYER GAME" / "2
/// PLAYER GAME" menu drawn over the title logo.
///
/// The vanilla stripe has a fixed structure: RLE blank-clear commands (which
/// erase the logo underneath the menu area) followed by raw text commands.
/// The editor preserves the clear commands verbatim and re-encodes only the
/// text from an editable grid. Text edits are refused if the re-encoded stripe
/// would exceed [`PLAYER_SELECT_STRIPE_MAX_SIZE`].
///
/// The menu owns tilemap rows [`MENU_FIRST_ROW`]`..=`[`MENU_LAST_ROW`]; paints
/// outside those rows belong to the title logo stripe.
pub const MENU_FIRST_ROW: usize = 15;
pub const MENU_LAST_ROW: usize = 21;

/// Split player-select commands into the preserved RLE clear commands and the
/// editable text commands (non-RLE).
pub fn split_player_select_commands(
    commands: &[TitleStripeCommand],
) -> (Vec<TitleStripeCommand>, Vec<TitleStripeCommand>) {
    let mut clears = Vec::new();
    let mut text = Vec::new();
    for cmd in commands {
        if cmd.rle {
            clears.push(cmd.clone());
        } else {
            text.push(cmd.clone());
        }
    }
    (clears, text)
}

/// Encode a player-select stripe from preserved clear commands and a text
/// grid. For each row in `MENU_FIRST_ROW..=MENU_LAST_ROW`, the span from the
/// first to the last non-blank cell is encoded as raw horizontal runs
/// (interior blanks are kept, matching the vanilla encoding), split at the
/// 32-column screen-block boundary; the clears run first so edited text draws
/// over the cleared logo area exactly like the hardware does.
pub fn encode_player_select_stripe(
    clears: &[TitleStripeCommand], text_grid: &TitleTileGrid,
) -> anyhow::Result<Vec<u8>> {
    let mut commands: Vec<TitleStripeCommand> = clears.to_vec();
    for y in MENU_FIRST_ROW..=MENU_LAST_ROW {
        let row = &text_grid.cells[y];
        // Two 32-column screen blocks per row; a DMA run cannot span the
        // block boundary.
        for block_x in 0..2 {
            let (b0, b1) = (block_x * 32, block_x * 32 + 32);
            let first = row[b0..b1].iter().position(|&w| w != TITLE_TILEMAP_BLANK);
            let last = row[b0..b1].iter().rposition(|&w| w != TITLE_TILEMAP_BLANK);
            if let (Some(f), Some(l)) = (first, last) {
                let (x0, x1) = (b0 + f, b0 + l);
                let tiles = row[x0..=x1].to_vec();
                let nbytes = tiles.len() * 2;
                commands.push(TitleStripeCommand {
                    vram_dest: TITLE_TILEMAP_VRAM_BASE + tilemap_64x64_word_offset(x0, y) as u16,
                    vertical: false,
                    rle: false,
                    nbytes,
                    tiles,
                });
            }
        }
    }
    let bytes = serialize_title_stripe(&commands);
    if bytes.len() > PLAYER_SELECT_STRIPE_MAX_SIZE {
        anyhow::bail!(
            "Encoded player select stripe is {} bytes, but the vanilla fixed slot is only {PLAYER_SELECT_STRIPE_MAX_SIZE} bytes; \
             erase some tiles to fit",
            bytes.len()
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod player_select_tests {
    use super::*;

    /// Minimal player-select stripe: one RLE clear + one text command.
    fn sample_menu_stripe() -> Vec<u8> {
        vec![
            0x51, 0xE5, 0x40, 0x05, // dest $51E5, RLE, 6 bytes = 3 blanks
            0xFC, 0x38, // $38FC
            0x52, 0x0A, 0x00, 0x03, // dest $520A, raw, 4 bytes
            0x6D, 0x31, 0x6F, 0x31, // $316D $316F
            0xFF,
        ]
    }

    #[test]
    fn split_separates_clears_and_text() {
        let cmds = parse_title_stripe(&sample_menu_stripe()).unwrap();
        let (clears, text) = split_player_select_commands(&cmds);
        assert_eq!(clears.len(), 1);
        assert!(clears[0].rle);
        assert_eq!(text.len(), 1);
        assert!(!text[0].rle);
    }

    #[test]
    fn encode_preserves_clears_and_text() {
        let cmds = parse_title_stripe(&sample_menu_stripe()).unwrap();
        let (clears, text) = split_player_select_commands(&cmds);
        let grid = TitleTileGrid::from_commands(&text);
        let bytes = encode_player_select_stripe(&clears, &grid).unwrap();
        // Same commands back (clears first, then the text run).
        assert_eq!(bytes, sample_menu_stripe());
    }

    #[test]
    fn encode_refuses_over_budget() {
        let cmds = parse_title_stripe(&sample_menu_stripe()).unwrap();
        let (clears, _) = split_player_select_commands(&cmds);
        // Fill the entire menu row range with non-blank tiles: far over the
        // 85-byte slot.
        let mut grid = TitleTileGrid::blank();
        for y in MENU_FIRST_ROW..=MENU_LAST_ROW {
            for x in 0..TITLE_TILEMAP_WIDTH {
                grid.cells[y][x] = 0x316D;
            }
        }
        assert!(encode_player_select_stripe(&clears, &grid).is_err());
    }

    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib --
    /// --ignored real_rom_player_select_stripe`.
    #[test]
    #[ignore]
    fn real_rom_player_select_stripe() {
        use crate::{snes_utils::addr::AddrPc, title_credits::PLAYER_SELECT_STRIPE_SNES};
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let data = std::fs::read(&rom_path).expect("read ROM");
        let pc = AddrPc::try_from_lorom(PLAYER_SELECT_STRIPE_SNES).unwrap().as_index();
        // Vanilla stripe is 85 bytes including the FF terminator.
        let bytes = &data[pc..pc + PLAYER_SELECT_STRIPE_MAX_SIZE];
        assert_eq!(bytes[PLAYER_SELECT_STRIPE_MAX_SIZE - 1], 0xFF);
        let cmds = parse_title_stripe(bytes).expect("parse player select stripe");
        let (clears, text) = split_player_select_commands(&cmds);
        assert!(!clears.is_empty(), "expected RLE clear commands");
        assert!(!text.is_empty(), "expected text commands");
        // Re-encoding the unmodified grid must reproduce the vanilla bytes.
        let grid = TitleTileGrid::from_commands(&text);
        let reencoded = encode_player_select_stripe(&clears, &grid).expect("re-encode");
        assert_eq!(reencoded, bytes, "re-encoded menu stripe differs from vanilla");
    }
}

// -------------------------------------------------------------------------------------------------
// Credits full-area editing (Lunar Magic v3.40 parity).
// -------------------------------------------------------------------------------------------------

/// First editable Layer-3 row in the credits scenes.
pub const CREDITS_L3_FIRST_ROW: usize = 0;
/// Last editable Layer-3 row in the credits scenes (vanilla text uses rows
/// 3-26 across the 13 scenes; 27 is the bottom of the 28-row viewport).
pub const CREDITS_L3_LAST_ROW: usize = 27;

/// True if a stripe command writes to Layer-3 tilemap VRAM ($5000-$5FFF word).
pub fn is_credits_l3_command(cmd: &TitleStripeCommand) -> bool {
    let dest = cmd.vram_dest as usize;
    (0x5000..0x6000).contains(&dest)
}

/// Split credits stripe commands into `(non_l3, l3)`. The non-L3 commands
/// (background RLE, etc.) are preserved verbatim on save; the L3 commands
/// are decoded into the editable grid.
pub fn split_credits_commands(commands: &[TitleStripeCommand]) -> (Vec<TitleStripeCommand>, Vec<TitleStripeCommand>) {
    let mut non_l3 = Vec::new();
    let mut l3 = Vec::new();
    for cmd in commands {
        if is_credits_l3_command(cmd) {
            l3.push(cmd.clone());
        } else {
            non_l3.push(cmd.clone());
        }
    }
    (non_l3, l3)
}

/// Encode a credits stripe from preserved non-L3 commands and an L3 text
/// grid. Each maximal contiguous non-blank run in rows
/// `CREDITS_L3_FIRST_ROW..=CREDITS_L3_LAST_ROW` is encoded as one raw
/// horizontal run (matching the vanilla encoding, which uses separate short
/// runs rather than full-row spans), split at the 32-column screen-block
/// boundary. The caller enforces the fixed slot budget.
pub fn encode_credits_stripe(
    non_l3: &[TitleStripeCommand], l3_grid: &TitleTileGrid, max_size: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut commands: Vec<TitleStripeCommand> = non_l3.to_vec();
    for y in CREDITS_L3_FIRST_ROW..=CREDITS_L3_LAST_ROW {
        let row = &l3_grid.cells[y];
        // Two 32-column screen blocks per row; a DMA run cannot span the
        // block boundary.
        for block_x in 0..2 {
            let mut x = block_x * 32;
            let end = x + 32;
            while x < end {
                if row[x] == TITLE_TILEMAP_BLANK {
                    x += 1;
                    continue;
                }
                let x0 = x;
                while x < end && row[x] != TITLE_TILEMAP_BLANK {
                    x += 1;
                }
                let tiles = row[x0..x].to_vec();
                let nbytes = tiles.len() * 2;
                commands.push(TitleStripeCommand {
                    vram_dest: TITLE_TILEMAP_VRAM_BASE + tilemap_64x64_word_offset(x0, y) as u16,
                    vertical: false,
                    rle: false,
                    nbytes,
                    tiles,
                });
            }
        }
    }
    let bytes = serialize_title_stripe(&commands);
    if bytes.len() > max_size {
        anyhow::bail!(
            "Encoded credits stripe is {} bytes, but the fixed slot is only {max_size} bytes; erase some tiles to fit",
            bytes.len()
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod credits_tests {
    use super::*;

    #[test]
    fn credits_l3_split() {
        // L3 command (dest $5000) vs non-L3 (dest $6000).
        let cmds = vec![
            TitleStripeCommand {
                vram_dest: 0x5000,
                vertical:  false,
                rle:       false,
                nbytes:    4,
                tiles:     vec![0x1234, 0x5678],
            },
            TitleStripeCommand {
                vram_dest: 0x6000,
                vertical:  false,
                rle:       true,
                nbytes:    100,
                tiles:     vec![0x38FC],
            },
        ];
        let (non_l3, l3) = split_credits_commands(&cmds);
        assert_eq!(non_l3.len(), 1);
        assert_eq!(l3.len(), 1);
        assert_eq!(l3[0].vram_dest, 0x5000);
    }

    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib -- --ignored real_rom_credits_stripes`.
    #[test]
    #[ignore]
    fn real_rom_credits_stripes() {
        use crate::{
            snes_utils::addr::AddrPc,
            title_credits::{ENEMY_NAME_COUNT, ENEMY_NAME_STRIPE_END_SNES, ENEMY_NAME_STRIPE_STARTS},
        };
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let data = std::fs::read(&rom_path).expect("read ROM");
        for i in 0..ENEMY_NAME_COUNT {
            let start = AddrPc::try_from_lorom(ENEMY_NAME_STRIPE_STARTS[i]).unwrap().as_index();
            let end = if i + 1 < ENEMY_NAME_COUNT {
                AddrPc::try_from_lorom(ENEMY_NAME_STRIPE_STARTS[i + 1]).unwrap().as_index()
            } else {
                AddrPc::try_from_lorom(ENEMY_NAME_STRIPE_END_SNES).unwrap().as_index()
            };
            let max_size = end - start;
            // Find the FF terminator.
            let mut len = 0;
            while len < max_size && data[start + len] != 0xFF {
                len += 1;
            }
            len += 1; // include FF
            let bytes = &data[start..start + len];
            let cmds = parse_title_stripe(bytes).expect("parse credits stripe");
            let (non_l3, l3) = split_credits_commands(&cmds);
            assert!(!l3.is_empty(), "scene {i}: expected L3 commands");
            // Decode L3 to grid and re-encode; must fit in the slot.
            let grid = TitleTileGrid::from_commands(&l3);
            let reencoded = encode_credits_stripe(&non_l3, &grid, max_size).expect("re-encode credits");
            assert!(reencoded.len() <= max_size, "scene {i}: re-encoded {} > slot {max_size}", reencoded.len());
            // Grid round-trip: decode the re-encoded L3 and compare.
            let recmds = parse_title_stripe(&reencoded).expect("parse re-encoded");
            let (_, re_l3) = split_credits_commands(&recmds);
            let regrid = TitleTileGrid::from_commands(&re_l3);
            assert_eq!(grid.cells, regrid.cells, "scene {i}: grid round-trip mismatch");
        }
    }
}
