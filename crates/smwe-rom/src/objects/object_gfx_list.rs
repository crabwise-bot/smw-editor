use thiserror::Error;

use crate::{objects::map16::Tile8x8, snes_utils::rom::Rom, AddrSnes, SnesSlice};

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
#[error("Could not parse GFX list at:\n- {0}")]
pub struct ObjectGfxListParseError(pub SnesSlice);

// -------------------------------------------------------------------------------------------------

const OBJECT_GFX_LIST: SnesSlice = SnesSlice::new(AddrSnes(0x00A92B), 26 * 4);

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
}
