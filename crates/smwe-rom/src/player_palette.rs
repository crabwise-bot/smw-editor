//! Player (Mario/Luigi) palettes — the four 10-color palettes the game DMAs to
//! CGRAM sprite palette row 8 whenever the player sprite tiles are (re)loaded.
//!
//! Ported from SMWDisX `bank_00.asm`: the `PlayerColors` table holds
//! `mario_normal.pal`, `luigi_normal.pal`, `mario_fire.pal`, `luigi_fire.pal`
//! back to back (10 colors / $14 bytes each). `DATA_00E2A2` selects among
//! them by `(Powerup << 1) | PlayerTurnLvl`, and `MarioGFXDMA` copies the
//! chosen $14 bytes to CGRAM $86 (colors $86-$8F of sprite palette row 8).
//!
//! Lunar Magic has no editor for these palettes, so this module backs the
//! Palette Editor's "Player Colors" section — a beyond-LM feature.

use crate::snes_utils::{
    addr::{AddrPc, AddrSnes},
    rom::Rom,
};

/// SNES address of the `PlayerColors` table (`bank_00.asm`).
pub const PLAYER_PALETTES_SNES: AddrSnes = AddrSnes(0x00B2C8);
/// The four palettes: Mario, Luigi, Mario (fire), Luigi (fire).
pub const PLAYER_PALETTE_COUNT: usize = 4;
/// Colors per palette (the DMA transfers $14 bytes = 10 words).
pub const PLAYER_PALETTE_COLORS: usize = 10;
pub const PLAYER_PALETTE_BYTES: usize = 0x14;

pub const PLAYER_PALETTE_NAMES: [&str; PLAYER_PALETTE_COUNT] =
    ["Mario", "Luigi", "Mario (Fire)", "Luigi (Fire)"];

/// The four player palettes as raw ABGR1555 words, matching what the Palette
/// Editor holds for the per-level palettes.
#[derive(Debug, Clone)]
pub struct PlayerPalettes {
    pub palettes: [[u16; PLAYER_PALETTE_COLORS]; PLAYER_PALETTE_COUNT],
}

impl PlayerPalettes {
    pub fn parse(rom: &Rom) -> anyhow::Result<Self> {
        let pc = AddrPc::try_from_lorom(PLAYER_PALETTES_SNES)
            .map_err(|e| anyhow::anyhow!("player palette addr conversion: {e}"))?
            .0 as usize;
        let end = pc + PLAYER_PALETTE_COUNT * PLAYER_PALETTE_BYTES;
        if end > rom.0.len() {
            anyhow::bail!("player palette table extends past end of ROM");
        }
        let mut palettes = [[0u16; PLAYER_PALETTE_COLORS]; PLAYER_PALETTE_COUNT];
        for (p, palette) in palettes.iter_mut().enumerate() {
            for (i, color) in palette.iter_mut().enumerate() {
                let off = pc + p * PLAYER_PALETTE_BYTES + i * 2;
                *color = rom.0[off] as u16 | ((rom.0[off + 1] as u16) << 8);
            }
        }
        Ok(Self { palettes })
    }

    pub fn color(&self, palette: usize, index: usize) -> Option<u16> {
        self.palettes.get(palette)?.get(index).copied()
    }

    pub fn set_color(&mut self, palette: usize, index: usize, color: u16) {
        if let Some(c) = self.palettes.get_mut(palette).and_then(|p| p.get_mut(index)) {
            *c = color;
        }
    }

    /// Write all four palettes back into a raw ROM image at `header_offset`.
    pub fn write_to(&self, rom_bytes: &mut [u8], header_offset: usize) -> anyhow::Result<()> {
        let pc = AddrPc::try_from_lorom(PLAYER_PALETTES_SNES)
            .map_err(|e| anyhow::anyhow!("player palette addr conversion: {e}"))?
            .as_index()
            + header_offset;
        for (p, palette) in self.palettes.iter().enumerate() {
            for (i, &color) in palette.iter().enumerate() {
                let off = pc + p * PLAYER_PALETTE_BYTES + i * 2;
                if off + 1 < rom_bytes.len() {
                    rom_bytes[off] = (color & 0xFF) as u8;
                    rom_bytes[off + 1] = (color >> 8) as u8;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected bytes: the four `col/misc/*_{normal,fire}.pal` files from
    /// SMWDisX converted by its `%incpal` macro (RGB888 → ABGR1555), in table
    /// order (Mario, Luigi, Mario fire, Luigi fire). Verified byte-for-byte
    /// against a real vanilla ROM dump (PC 0x32C8) on 2026-09-11.
    const EXPECTED: [[u16; PLAYER_PALETTE_COLORS]; PLAYER_PALETTE_COUNT] = [
        [0x635f, 0x581d, 0x000a, 0x391f, 0x44c4, 0x4e08, 0x6770, 0x30b6, 0x35df, 0x03ff],
        [0x4f3f, 0x581d, 0x1140, 0x3fe0, 0x3c07, 0x7cae, 0x7db3, 0x2f00, 0x165f, 0x03ff],
        [0x635f, 0x581d, 0x2529, 0x7fff, 0x0008, 0x0017, 0x001f, 0x577b, 0x0ddf, 0x03ff],
        [0x3b1f, 0x581d, 0x2529, 0x7fff, 0x1140, 0x01e0, 0x02e0, 0x577b, 0x0ddf, 0x03ff],
    ];

    #[test]
    fn set_color_round_trips_without_touching_neighbors() {
        let mut pp = PlayerPalettes { palettes: [[0u16; PLAYER_PALETTE_COLORS]; PLAYER_PALETTE_COUNT] };
        pp.set_color(0, 3, 0x7FFF);
        assert_eq!(pp.color(0, 3), Some(0x7FFF));
        assert_eq!(pp.color(0, 2), Some(0));
        assert_eq!(pp.color(1, 3), Some(0));
        // Out of range is a no-op, not a panic.
        pp.set_color(9, 0, 0x7FFF);
        pp.set_color(0, 40, 0x7FFF);
        assert_eq!(pp.color(9, 0), None);
        assert_eq!(pp.color(0, 40), None);
    }

    #[test]
    fn write_to_round_trips_through_parse() {
        let pp = PlayerPalettes { palettes: EXPECTED };
        let mut fake_rom = vec![0u8; 0x80000];
        pp.write_to(&mut fake_rom, 0).unwrap();
        // Re-parse from a Rom wrapping those bytes.
        let rom = Rom(fake_rom.into());
        let parsed = PlayerPalettes::parse(&rom).unwrap();
        assert_eq!(parsed.palettes, EXPECTED);
    }

    /// Run with `ROM_PATH=~/workspace/smw-editor/smw.smc cargo test -p smwe-rom
    /// --lib -- --ignored real_rom_player_palettes`.
    #[test]
    #[ignore]
    fn real_rom_player_palettes() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = crate::SmwRom::from_file(rom_path).expect("parse ROM");
        let parsed = PlayerPalettes::parse(&rom.rom).expect("parse player palettes");
        assert_eq!(parsed.palettes, EXPECTED, "player palettes must match SMWDisX PlayerColors");
        // Spot-check the famous Mario red: palette 0 holds the red shirt.
        let reds: Vec<u16> = parsed.palettes[0].iter().copied().filter(|&c| {
            let r = c & 0x1F;
            let g = (c >> 5) & 0x1F;
            r > 0x18 && g < 0x08
        }).collect();
        assert!(!reds.is_empty(), "Mario palette should contain a strong red");
        println!("Mario reds: {reds:04X?}");
    }
}
