//! Overworld starting positions for Mario and Luigi (Lunar Magic v1.60/v1.90
//! parity: v1.60 made Mario's starting position settable by moving the Mario
//! marker; v1.90 made Luigi's settable separately).
//!
//! The table is SNES `$009EF0`, 22 bytes, copied verbatim by `InitSaveData`
//! into the save buffer for new games (SMWDisX `bank_00.asm`,
//! `InitPlayerOverworldData`):
//!
//! ```text
//! [0]    Mario submap
//! [1]    Luigi submap
//! [2..4] Mario initial animation (word, vanilla = 2)
//! [4..6] Luigi initial animation (word, vanilla = 2)
//! [6..8] Mario pixel X (word)
//! [8..10] Mario pixel Y (word)
//! [10..12] Luigi pixel X (word)
//! [12..14] Luigi pixel Y (word)
//! [14..16] Mario tile X (word) = pixel X >> 4
//! [16..18] Mario tile Y (word) = pixel Y >> 4
//! [18..20] Luigi tile X (word) = pixel X >> 4
//! [20..22] Luigi tile Y (word) = pixel Y >> 4
//! ```
//!
//! The tile coordinates are 16×16-tile units of the main-map pixel space
//! (the pixel coordinates are the tile's center: `pixel = tile * 16 + 8`).
//! Editing is in place — 22 fixed bytes — so no relocation patch is needed
//! and untouched ROMs stay byte-identical.

use crate::snes_utils::addr::{AddrPc, AddrSnes};

/// SNES address of the 22-byte `InitPlayerOverworldData` table.
pub const START_POSITIONS_SNES: AddrSnes = AddrSnes(0x009EF0);
/// Fixed length of the table in bytes.
pub const START_POSITIONS_LEN: usize = 22;

/// One player's overworld starting position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerStart {
    /// Which submap the player appears on (0..=6, see
    /// [`SUBMAP_NAMES`][crate::overworld::SUBMAP_NAMES]).
    pub submap:  u8,
    /// Horizontal pixel position on the main-map pixel space.
    pub pixel_x: u16,
    /// Vertical pixel position on the main-map pixel space.
    pub pixel_y: u16,
    /// Horizontal 16×16-tile position (`pixel_x >> 4`).
    pub tile_x:  u16,
    /// Vertical 16×16-tile position (`pixel_y >> 4`).
    pub tile_y:  u16,
}

impl PlayerStart {
    /// Move to 16×16-tile `(x, y)`, recomputing the pixel coordinates so the
    /// position stays on the tile's center (the vanilla invariant:
    /// `pixel = tile * 16 + 8`, `tile = pixel >> 4`).
    pub fn set_tile(&mut self, x: u16, y: u16) {
        self.tile_x = x;
        self.tile_y = y;
        self.pixel_x = x.wrapping_mul(16).wrapping_add(8);
        self.pixel_y = y.wrapping_mul(16).wrapping_add(8);
    }

    /// Move to raw pixel `(x, y)`, recomputing the tile coordinates as the
    /// game does (`tile = pixel >> 4`).
    pub fn set_pixel(&mut self, x: u16, y: u16) {
        self.pixel_x = x;
        self.pixel_y = y;
        self.tile_x = x >> 4;
        self.tile_y = y >> 4;
    }
}

/// Mario's and Luigi's overworld starting positions, plus the two animation
/// words the table carries alongside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverworldStartPositions {
    pub mario:      PlayerStart,
    pub luigi:      PlayerStart,
    /// Mario's initial animation word (`[2..4]`, vanilla = 2).
    pub mario_anim: u16,
    /// Luigi's initial animation word (`[4..6]`, vanilla = 2).
    pub luigi_anim: u16,
}

impl OverworldStartPositions {
    /// Parse the 22-byte table from ROM bytes (`header_offset` = `0x200` if
    /// the ROM has an SMC header, else `0`).
    pub fn parse(rom: &[u8], header_offset: usize) -> anyhow::Result<Self> {
        let pc = AddrPc::try_from_lorom(START_POSITIONS_SNES)
            .map_err(|e| anyhow::anyhow!("start-positions addr conversion: {e}"))?
            .0 as usize
            + header_offset;
        let bytes = rom
            .get(pc..pc + START_POSITIONS_LEN)
            .ok_or_else(|| anyhow::anyhow!("start-positions table extends past end of ROM"))?;
        Ok(Self::decode(bytes))
    }

    /// Decode from exactly [`START_POSITIONS_LEN`] bytes.
    pub fn decode(bytes: &[u8]) -> Self {
        let w = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let player = |submap: u8, px: usize, py: usize, tx: usize, ty: usize| PlayerStart {
            submap,
            pixel_x: w(px),
            pixel_y: w(py),
            tile_x: w(tx),
            tile_y: w(ty),
        };
        Self {
            mario:      player(bytes[0], 6, 8, 14, 16),
            luigi:      player(bytes[1], 10, 12, 18, 20),
            mario_anim: w(2),
            luigi_anim: w(4),
        }
    }

    /// Encode to exactly [`START_POSITIONS_LEN`] bytes.
    pub fn encode(&self) -> [u8; START_POSITIONS_LEN] {
        let mut out = [0u8; START_POSITIONS_LEN];
        out[0] = self.mario.submap;
        out[1] = self.luigi.submap;
        let mut w = |i: usize, v: u16| out[i..i + 2].copy_from_slice(&v.to_le_bytes());
        w(2, self.mario_anim);
        w(4, self.luigi_anim);
        w(6, self.mario.pixel_x);
        w(8, self.mario.pixel_y);
        w(10, self.luigi.pixel_x);
        w(12, self.luigi.pixel_y);
        w(14, self.mario.tile_x);
        w(16, self.mario.tile_y);
        w(18, self.luigi.tile_x);
        w(20, self.luigi.tile_y);
        out
    }

    /// Write the table back into `rom_bytes` in place.
    pub fn apply_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> anyhow::Result<()> {
        let pc = AddrPc::try_from_lorom(START_POSITIONS_SNES)
            .map_err(|e| anyhow::anyhow!("start-positions addr conversion: {e}"))?
            .0 as usize
            + header_offset;
        let dst = rom_bytes
            .get_mut(pc..pc + START_POSITIONS_LEN)
            .ok_or_else(|| anyhow::anyhow!("start-positions write range out of bounds"))?;
        dst.copy_from_slice(&self.encode());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact vanilla bytes at SNES `$009EF0` (PC `0x1EF0`).
    fn vanilla_bytes() -> [u8; START_POSITIONS_LEN] {
        [
            0x01, 0x01, 0x02, 0x00, 0x02, 0x00, 0x68, 0x00, 0x78, 0x00, 0x68, 0x00, 0x78, 0x00, 0x06, 0x00, 0x07, 0x00,
            0x06, 0x00, 0x07, 0x00,
        ]
    }

    #[test]
    fn decode_vanilla_values() {
        let pos = OverworldStartPositions::decode(&vanilla_bytes());
        assert_eq!(pos.mario.submap, 1);
        assert_eq!(pos.luigi.submap, 1);
        assert_eq!(pos.mario_anim, 2);
        assert_eq!(pos.luigi_anim, 2);
        assert_eq!((pos.mario.pixel_x, pos.mario.pixel_y), (0x68, 0x78));
        assert_eq!((pos.luigi.pixel_x, pos.luigi.pixel_y), (0x68, 0x78));
        assert_eq!((pos.mario.tile_x, pos.mario.tile_y), (6, 7));
        assert_eq!((pos.luigi.tile_x, pos.luigi.tile_y), (6, 7));
    }

    #[test]
    fn round_trip_encode_decode() {
        let pos = OverworldStartPositions::decode(&vanilla_bytes());
        assert_eq!(pos.encode(), vanilla_bytes());
    }

    #[test]
    fn set_tile_keeps_pixel_consistent() {
        let mut pos = OverworldStartPositions::decode(&vanilla_bytes());
        pos.mario.set_tile(10, 5);
        assert_eq!((pos.mario.pixel_x, pos.mario.pixel_y), (10 * 16 + 8, 5 * 16 + 8));
        assert_eq!(pos.mario.pixel_x >> 4, 10);
        assert_eq!(pos.mario.pixel_y >> 4, 5);
        // Luigi untouched.
        assert_eq!((pos.luigi.pixel_x, pos.luigi.pixel_y), (0x68, 0x78));
    }

    #[test]
    fn set_pixel_derives_tile() {
        let mut pos = OverworldStartPositions::decode(&vanilla_bytes());
        pos.luigi.set_pixel(0x120, 0x90);
        assert_eq!((pos.luigi.tile_x, pos.luigi.tile_y), (0x12, 0x09));
    }

    #[test]
    fn apply_writes_in_place() {
        let before_pc = AddrPc::try_from_lorom(START_POSITIONS_SNES).unwrap().0 as usize;
        let mut rom = vec![0xAAu8; before_pc + START_POSITIONS_LEN + 4];
        rom[before_pc..before_pc + START_POSITIONS_LEN].copy_from_slice(&vanilla_bytes());

        let mut pos = OverworldStartPositions::parse(&rom, 0).unwrap();
        pos.mario.submap = 0;
        pos.mario.set_tile(3, 9);
        pos.apply_to_rom(&mut rom, 0).unwrap();

        let again = OverworldStartPositions::parse(&rom, 0).unwrap();
        assert_eq!(again, pos);
        assert_eq!(again.mario.submap, 0);
        assert_eq!((again.mario.tile_x, again.mario.tile_y), (3, 9));
        // Neighbors untouched.
        assert_eq!(rom[before_pc + START_POSITIONS_LEN], 0xAA);
        assert_eq!(rom[before_pc - 1], 0xAA);
    }

    /// Parse the real ROM and check the table against the disassembly
    /// (`InitPlayerOverworldData`, SMWDisX `bank_00.asm`) and the
    /// byte-identical transcription. Run with
    /// `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib -- --ignored
    /// real_rom_start_positions`.
    #[test]
    #[ignore]
    fn real_rom_start_positions() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let raw = std::fs::read(rom_path).expect("read ROM");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

        let pos = OverworldStartPositions::parse(&rom_bytes, 0).expect("start positions parse");
        assert_eq!(pos.encode(), vanilla_bytes(), "real ROM table differs from the disassembly transcription");
        assert_eq!((pos.mario.pixel_x, pos.mario.pixel_y), (0x68, 0x78));
        assert_eq!((pos.mario.tile_x, pos.mario.tile_y), (6, 7));
        // Byte-identical write-back on an untouched table.
        let mut copy = rom_bytes.clone();
        pos.apply_to_rom(&mut copy, 0).unwrap();
        assert_eq!(copy, rom_bytes);
    }
}
