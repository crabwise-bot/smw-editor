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
//! Note the vertical stride (32 words) against the 64-wide tilemap: one
//! vertical command draws two interleaved columns 32 apart (e.g. the logo's
//! left edge lives in columns 0–1 *and* 32–33). The parser reproduces this
//! exactly; commands are applied in order so overlapping writes compose like
//! the hardware DMA does.
//!
//! RLE commands are rejected: the vanilla title stripe contains none, and the
//! editor never emits them.
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

use crate::title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE;

/// Width/height of the title Layer 3 tilemap in tiles (`Size_64x64`).
pub const TITLE_TILEMAP_WIDTH: usize = 64;
pub const TITLE_TILEMAP_HEIGHT: usize = 64;
/// VRAM word address of the Layer 3 tilemap (`VRam_L3Tilemap`).
pub const TITLE_TILEMAP_VRAM_BASE: u16 = 0x5000;
/// Tile word `ClearOutLayer3` fills the Layer 3 tilemap with before the
/// title stripe uploads (`!EmptyTile` in SMWDisX `constants.asm`). Cells
/// holding this value are "blank" and skipped by the encoder.
pub const TITLE_TILEMAP_BLANK: u16 = 0x38FC;

/// One parsed stripe-image command (see `LoadStripeImage` in SMWDisX
/// `bank_00.asm`).
#[derive(Debug, Clone)]
pub struct TitleStripeCommand {
    /// VRAM word destination.
    pub vram_dest: u16,
    /// True = vertical (32-word stride), false = horizontal (1-word stride).
    pub vertical: bool,
    /// Payload tile words, little-endian in the stripe.
    pub tiles: Vec<u16>,
}

/// Parse raw stripe-image bytes into commands.
///
/// Stops at the first byte with bit 7 set (the `$FF` terminator); any bytes
/// after it are ignored. Errors on truncated headers/payloads, odd payload
/// lengths, and RLE commands.
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
        if flags & 0x40 != 0 {
            anyhow::bail!("RLE title stripe commands are not supported (offset {i:#X})");
        }
        let nbytes = (((flags & 0x3F) as usize) << 8 | bytes[i + 3] as usize) + 1;
        if nbytes % 2 != 0 {
            anyhow::bail!("odd title stripe payload length {nbytes} at offset {i:#X}");
        }
        let payload_end = i + 4 + nbytes;
        if payload_end > bytes.len() {
            anyhow::bail!("truncated title stripe payload at offset {i:#X}");
        }
        let tiles = bytes[i + 4..payload_end]
            .chunks_exact(2)
            .map(|w| u16::from_le_bytes([w[0], w[1]]))
            .collect();
        commands.push(TitleStripeCommand { vram_dest, vertical: flags & 0x80 != 0, tiles });
        i = payload_end;
    }
    Ok(commands)
}

/// Serialize commands back to stripe bytes, terminated with `$FF`.
pub fn serialize_title_stripe(commands: &[TitleStripeCommand]) -> Vec<u8> {
    let mut out = Vec::new();
    for cmd in commands {
        out.extend_from_slice(&cmd.vram_dest.to_be_bytes());
        let nbytes = cmd.tiles.len() * 2;
        debug_assert!(nbytes >= 1 && nbytes <= 0x4000);
        let mut flags = ((nbytes - 1) >> 8) as u8 & 0x3F;
        if cmd.vertical {
            flags |= 0x80;
        }
        out.push(flags);
        out.push(((nbytes - 1) & 0xFF) as u8);
        for &tile in &cmd.tiles {
            out.extend_from_slice(&tile.to_le_bytes());
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
    /// words (vertical) per tile. Later commands overwrite earlier words, so
    /// overlapping commands compose exactly like the hardware DMA does.
    pub fn from_commands(commands: &[TitleStripeCommand]) -> Self {
        let mut cells = [[TITLE_TILEMAP_BLANK; TITLE_TILEMAP_WIDTH]; TITLE_TILEMAP_HEIGHT];
        let base = TITLE_TILEMAP_VRAM_BASE as usize;
        // Word offset of the last tilemap word from the base.
        let max_word = TITLE_TILEMAP_WIDTH * TITLE_TILEMAP_HEIGHT;
        for cmd in commands {
            let mut dest = cmd.vram_dest as usize;
            let stride = if cmd.vertical { 32 } else { 1 };
            for &tile in &cmd.tiles {
                if dest >= base {
                    let wo = dest - base;
                    if wo < max_word {
                        cells[wo / TITLE_TILEMAP_WIDTH][wo % TITLE_TILEMAP_WIDTH] = tile;
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
    /// Errors if the encoding would exceed
    /// [`crate::title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE`] — over-budget
    /// edits are refused, never silently truncated.
    pub fn to_stripe_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut commands = Vec::new();
        for (y, row) in self.cells.iter().enumerate() {
            let mut x = 0;
            while x < TITLE_TILEMAP_WIDTH {
                if row[x] == TITLE_TILEMAP_BLANK {
                    x += 1;
                    continue;
                }
                let x0 = x;
                while x < TITLE_TILEMAP_WIDTH && row[x] != TITLE_TILEMAP_BLANK {
                    x += 1;
                }
                commands.push(TitleStripeCommand {
                    vram_dest: TITLE_TILEMAP_VRAM_BASE + (y * TITLE_TILEMAP_WIDTH + x0) as u16,
                    vertical: false,
                    tiles: row[x0..x].to_vec(),
                });
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
    /// (10,0), 32-word stride → (10,0) and (42,0) in the 64-wide map).
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
    fn parse_rejects_rle() {
        let mut b = sample_stripe();
        b[2] = 0x40; // set the RLE bit in the first command's flags byte
        assert!(parse_title_stripe(&b).is_err());
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
        // vertical command: (10,0) and (42,0) — 32-word stride in 64-wide map
        assert_eq!(grid.cells[0][10], 0x3C98);
        assert_eq!(grid.cells[0][42], 0x3CA9);
        // untouched cells are blank
        assert_eq!(grid.cells[0][3], TITLE_TILEMAP_BLANK);
        assert_eq!(grid.cells[1][10], TITLE_TILEMAP_BLANK);
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
    use crate::title_credits::TITLE_SCREEN_STRIPE_SNES;
    use crate::{snes_utils::addr::AddrPc, SmwRom};

    fn real_stripe_bytes(rom_path: &str) -> Vec<u8> {
        let raw = std::fs::read(rom_path).expect("read ROM");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
        let start = AddrPc::try_from_lorom(TITLE_SCREEN_STRIPE_SNES).unwrap().as_index();
        let slot = &rom_bytes[start..start + TITLE_SCREEN_STRIPE_MAX_SIZE];
        let end = slot
            .iter()
            .position(|&b| b == 0xFF)
            .map(|p| p + 1)
            .unwrap_or(slot.len());
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
        // Row 0 is the 64-wide top of the "SUPER MARIO WORLD" logo:
        // palette-3 tiles across the full row.
        for x in 0..64 {
            let w = grid.cells[0][x];
            assert_ne!(w, TITLE_TILEMAP_BLANK, "row 0 col {x} should be logo");
            assert_eq!((w >> 10) & 7, 3, "row 0 col {x}: expected palette 3, got {w:#06X}");
        }
        // The logo's vertical strips live in columns 0-1 and 32-33.
        assert_ne!(grid.cells[1][0], TITLE_TILEMAP_BLANK);
        assert_ne!(grid.cells[1][32], TITLE_TILEMAP_BLANK);

        // Normalized re-encode is DMA-equivalent and fits the budget.
        let reencoded = grid.to_stripe_bytes().expect("re-encode fits budget");
        println!(
            "normalized re-encode: {} bytes / {TITLE_SCREEN_STRIPE_MAX_SIZE} budget",
            reencoded.len()
        );
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
        let grid = TitleTileGrid::from_stripe(&rom.title_credits.title_screen_stripe)
            .expect("parse title stripe via SmwRom");
        let nonzero = grid
            .cells
            .iter()
            .flatten()
            .filter(|&&w| w != TITLE_TILEMAP_BLANK)
            .count();
        println!("non-blank cells: {nonzero}");
        assert!(nonzero > 300, "expected a densely-drawn logo, found {nonzero} cells");
    }
}

