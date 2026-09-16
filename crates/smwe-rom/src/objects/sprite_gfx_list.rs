use thiserror::Error;

use crate::{snes_utils::rom::Rom, AddrSnes, SnesSlice};

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
#[error("Could not parse sprite GFX list at:\n- {0}")]
pub struct SpriteGfxListParseError(pub SnesSlice);

// -------------------------------------------------------------------------------------------------

/// SPRITEGFXLIST ($00A8C3): 26 rows of 4 GFX file numbers, indexed by the
/// level header's Sprite GFX nibble ($7E192B). Each row holds the four sprite
/// GFX files the game's `UploadSpriteGFX` routine (`bank_00.asm`) uploads for
/// that sprite tileset.
///
/// The address is pinned against the real ROM, not the disassembly's symbol
/// label ($00A8C8): the label is 5 bytes off, exactly like OBJECTGFXLIST's
/// ($00A930 vs the real $00A92B). SPRITEGFXLIST is exactly the 26*4 bytes
/// preceding OBJECTGFXLIST ($00A92B - 0x68 = $00A8C3), and all 26 rows match
/// the disassembly's table byte-for-byte (verified in `sprite_gfx_list_table`).
///
/// Upload order (traced from `UploadSpriteGFX` + `DATA_00A9D2` = $78,$70,$68,$60):
/// row byte 0 lands at VRAM tiles 0x780-0x7FF (SP1), byte 1 at 0x700-0x77F
/// (SP2), byte 2 at 0x680-0x6FF (SP3), byte 3 at 0x600-0x67F (SP4).
const SPRITE_GFX_LIST: SnesSlice = SnesSlice::new(AddrSnes(0x00A8C3), 26 * 4);

/// Number of sprite tileset rows in SPRITEGFXLIST.
pub const SPRITE_TILESET_COUNT: usize = 26;

/// VRAM 8x8-tile bases where `UploadSpriteGFX` uploads row bytes 0..=3
/// (SP1..SP4): 0x780, 0x700, 0x680, 0x600.
pub const SPRITE_SLOT_VRAM_BASES: [u16; 4] = [0x780, 0x700, 0x680, 0x600];

/// LM-style slot labels for the four sprite GFX slots, in table row order.
pub const SPRITE_SLOT_NAMES: [&str; 4] = ["SP1", "SP2", "SP3", "SP4"];

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct SpriteGfxList {
    gfx_file_nums: Vec<u8>,
}

// -------------------------------------------------------------------------------------------------

impl SpriteGfxList {
    pub fn parse(rom: &Rom) -> Result<Self, SpriteGfxListParseError> {
        let gfx_file_nums =
            rom.slice_lorom(SPRITE_GFX_LIST).map_err(|_| SpriteGfxListParseError(SPRITE_GFX_LIST))?.to_vec();
        Ok(Self { gfx_file_nums })
    }

    /// The four GFX file numbers for sprite tileset `tileset` (0..26), in
    /// table (row) order: [SP1, SP2, SP3, SP4].
    pub fn files_for_sprite_tileset(&self, tileset: usize) -> [usize; 4] {
        let base = tileset * 4;
        [
            self.gfx_file_nums[base] as usize,
            self.gfx_file_nums[base + 1] as usize,
            self.gfx_file_nums[base + 2] as usize,
            self.gfx_file_nums[base + 3] as usize,
        ]
    }

    /// VRAM 8x8-tile base where `UploadSpriteGFX` uploads row byte `i`
    /// (0..=3): 0 -> 0x780, 1 -> 0x700, 2 -> 0x680, 3 -> 0x600.
    pub const fn vram_tile_base_for_row_byte(i: usize) -> u16 {
        SPRITE_SLOT_VRAM_BASES[i]
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_slot_vram_bases_match_upload_order() {
        // DATA_00A9D2 = $78,$70,$68,$60: row byte 0 lands highest.
        assert_eq!(SpriteGfxList::vram_tile_base_for_row_byte(0), 0x780);
        assert_eq!(SpriteGfxList::vram_tile_base_for_row_byte(1), 0x700);
        assert_eq!(SpriteGfxList::vram_tile_base_for_row_byte(2), 0x680);
        assert_eq!(SpriteGfxList::vram_tile_base_for_row_byte(3), 0x600);
    }

    #[test]
    fn sprite_gfx_list_size_matches_object_gfx_list_layout() {
        // The table must be exactly the 26*4 bytes preceding OBJECTGFXLIST.
        assert_eq!(SPRITE_GFX_LIST.begin.0, 0x00A8C3);
        assert_eq!(SPRITE_GFX_LIST.size, SPRITE_TILESET_COUNT * 4);
        assert_eq!(SPRITE_GFX_LIST.begin.0 + SPRITE_GFX_LIST.size as u32, 0x00A92B);
    }
}

#[cfg(test)]
mod real_rom_tests {
    use super::*;

    /// Pin the table address and row 0 against the disassembly: row 0 is the
    /// "Forest" row ($00,$01,$13,$02). Every referenced file must exist; the
    /// 16 rows addressable by a level header nibble must each hold at least
    /// 0x80 tiles (one upload slot's worth) — the game uploads a full 0x80-tile
    /// slot per row byte. Rows 16-25 are not reachable from a vanilla level
    /// header nibble (row 25 names GFX file $30, which is smaller than a full
    /// slot), so they are only checked for existence. Run with
    /// `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib -- --ignored
    /// sprite_gfx_list_table`.
    #[test]
    #[ignore]
    fn sprite_gfx_list_table() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = crate::SmwRom::from_file(rom_path).expect("parse ROM");
        assert_eq!(
            rom.gfx.sprite_gfx_list.files_for_sprite_tileset(0),
            [0x00, 0x01, 0x13, 0x02],
            "sprite tileset 0 must be the 'Forest' row"
        );
        for tileset in 0..SPRITE_TILESET_COUNT {
            for file_num in rom.gfx.sprite_gfx_list.files_for_sprite_tileset(tileset) {
                let n_tiles = rom.gfx.files.get(file_num).map(|f| f.tiles.len()).unwrap_or(0);
                assert!(n_tiles > 0, "sprite tileset {tileset}: file {file_num:02X} missing");
                if tileset < 16 {
                    assert!(
                        n_tiles >= 0x80,
                        "sprite tileset {tileset}: file {file_num:02X} too small for an upload slot"
                    );
                }
            }
        }
    }
}
