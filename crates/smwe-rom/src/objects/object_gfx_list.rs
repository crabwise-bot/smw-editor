use thiserror::Error;

use crate::{objects::map16::Tile8x8, snes_utils::rom::Rom, AddrSnes, SnesSlice};

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
#[error("Could not parse GFX list at:\n- {0}")]
pub struct ObjectGfxListParseError(pub SnesSlice);

// -------------------------------------------------------------------------------------------------

const OBJECT_GFX_LIST: SnesSlice = SnesSlice::new(AddrSnes(0x00A92B), 26 * 4);

// -------------------------------------------------------------------------------------------------

/// Number of FG/BG tileset rows in OBJECTGFXLIST.
pub const OBJECT_TILESET_COUNT: usize = 26;

/// LM-style slot labels for the four FG/BG GFX slots, in table row order.
pub const OBJECT_SLOT_NAMES: [&str; 4] = ["FG1", "FG2", "FG3", "BG1"];

/// VRAM 8x8-tile ranges for the four FG/BG upload slots, in row order
/// (the game's level upload loop, `CODE_00AA35`, writes each row byte's
/// 0x80 tiles contiguously starting at tile 0x000).
pub const OBJECT_SLOT_VRAM_RANGES: [(u16, u16); 4] = [(0x000, 0x07F), (0x080, 0x0FF), (0x100, 0x17F), (0x180, 0x1FF)];

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct ObjectGfxList {
    gfx_file_nums: Vec<u8>,
}

// -------------------------------------------------------------------------------------------------

impl ObjectGfxList {
    pub fn parse(rom: &Rom) -> Result<Self, ObjectGfxListParseError> {
        let gfx_file_nums =
            rom.slice_lorom(OBJECT_GFX_LIST).map_err(|_| ObjectGfxListParseError(OBJECT_GFX_LIST))?.to_vec();
        Ok(Self { gfx_file_nums })
    }

    pub fn gfx_file_for_object_tile(&self, tile: Tile8x8, tileset: usize) -> usize {
        let idx = (tileset * 4) + tile.layer();
        self.gfx_file_nums[idx] as usize
    }

    /// The four GFX file numbers for FG/BG tileset `tileset` (0..26), in
    /// table (row) order: [FG1, FG2, FG3, BG1].
    pub fn files_for_object_tileset(&self, tileset: usize) -> [usize; 4] {
        let base = tileset * 4;
        [
            self.gfx_file_nums[base] as usize,
            self.gfx_file_nums[base + 1] as usize,
            self.gfx_file_nums[base + 2] as usize,
            self.gfx_file_nums[base + 3] as usize,
        ]
    }
}
