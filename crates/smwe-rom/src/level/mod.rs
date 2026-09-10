use nom::{
    combinator::map,
    multi::count,
    number::complete::{le_u16, le_u24},
};
use thiserror::Error;

pub use self::{
    background::{BackgroundData, BackgroundTileID},
    headers::{PrimaryHeader, SecondaryHeader, SpriteHeader, PRIMARY_HEADER_SIZE, SPRITE_HEADER_SIZE},
    object_layer::ObjectLayer,
    sprite_layer::SpriteLayer,
};
use crate::{
    compression::DecompressionError,
    snes_utils::{
        addr::AddrSnes,
        rom::{parse_bytes, Rom},
        rom_slice::SnesSlice,
    },
    RomError,
};

pub mod background;
pub mod headers;
pub mod object_layer;
pub mod secondary_entrance;
pub mod sprite_layer;

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum LevelParseError {
    #[error("Reading address of Layer1:\n- {0}")]
    Layer1AddressRead(RomError),
    #[error("Reading address of Layer2:\n- {0}")]
    Layer2AddressRead(RomError),
    #[error("Reading address of Sprite data:\n- {0}")]
    SpriteAddressRead(RomError),

    #[error("Isolating Layer2 data:\n- {0}")]
    Layer2Isolate(RomError),

    #[error("Reading Primary Header:\n- {0}")]
    PrimaryHeaderRead(RomError),
    #[error("Reading Secondary Header:\n- {0}")]
    SecondaryHeaderRead(RomError),
    #[error("Reading Sprite Header:\n- {0}")]
    SpriteHeaderRead(RomError),

    #[error("Reading Layer1 object data:\n- {0}")]
    Layer1Read(RomError),
    #[error("Parsing Layer2 object data:\n- {0}")]
    Layer2Read(RomError),
    #[error("Reading Layer2 background:\n- {0}")]
    Layer2BackgroundRead(DecompressionError),
    #[error("Reading Sprite data:\n- {0}")]
    SpriteRead(RomError),
}

// -------------------------------------------------------------------------------------------------

pub const LEVEL_COUNT: usize = 0x200;

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Layer2Data {
    Background(BackgroundData),
    Objects(ObjectLayer),
}

#[derive(Debug, Clone)]
pub struct Level {
    pub primary_header:   PrimaryHeader,
    pub secondary_header: SecondaryHeader,
    pub sprite_header:    SpriteHeader,
    pub layer1:           ObjectLayer,
    pub layer2:           Layer2Data,
    pub sprite_layer:     SpriteLayer,
}

// -------------------------------------------------------------------------------------------------

impl Level {
    pub fn parse(rom: &Rom, level_num: u32) -> Result<Self, LevelParseError> {
        let (primary_header, layer1) = Self::parse_ph_and_l1(rom, level_num)?;
        let layer2 = Self::parse_l2(rom, level_num)?;
        let (sprite_header, sprite_layer) = Self::parse_sh_and_sl(rom, level_num)?;
        let secondary_header =
            SecondaryHeader::read_from_rom(rom, level_num).map_err(LevelParseError::SecondaryHeaderRead)?;

        Ok(Level { primary_header, secondary_header, sprite_header, layer1, layer2, sprite_layer })
    }

    fn parse_ph_and_l1(rom: &Rom, level_num: u32) -> Result<(PrimaryHeader, ObjectLayer), LevelParseError> {
        let l1_ptr_slice = SnesSlice::new(AddrSnes(0x05E000), 0x200 * 3);
        let ph_addr = rom
            .parse_lorom(l1_ptr_slice, count(map(le_u24, AddrSnes), 0x200))
            .map_err(LevelParseError::Layer1AddressRead)?[level_num as usize];

        let primary_header = {
            let ph_slice = SnesSlice::new(ph_addr, PRIMARY_HEADER_SIZE);
            PrimaryHeader::new(rom.slice_lorom(ph_slice).map_err(LevelParseError::PrimaryHeaderRead)?)
        };

        let layer1 = {
            let bytes = rom.slice_from(ph_addr + PRIMARY_HEADER_SIZE as u32).map_err(LevelParseError::Layer1Read)?;
            parse_bytes(bytes, ObjectLayer::parse).map_err(LevelParseError::Layer1Read)?.0
        };

        Ok((primary_header, layer1))
    }

    fn parse_l2(rom: &Rom, level_num: u32) -> Result<Layer2Data, LevelParseError> {
        const LAYER2_DATA: AddrSnes = AddrSnes(0x05E600);

        let l2_addr_slice = SnesSlice::new(LAYER2_DATA + (3 * level_num), 3);
        let l2_ptr =
            rom.parse_lorom(l2_addr_slice, map(le_u24, AddrSnes)).map_err(LevelParseError::Layer2AddressRead)?;

        if l2_ptr.bank() == 0xFF {
            let bytes = rom.slice_from(l2_ptr.with_bank(0x0C)).map_err(LevelParseError::Layer2Isolate)?;
            let (background, _) = BackgroundData::read_from(bytes).map_err(LevelParseError::Layer2BackgroundRead)?;
            Ok(Layer2Data::Background(background))
        } else {
            let bytes = rom.slice_from(l2_ptr + PRIMARY_HEADER_SIZE as u32).map_err(LevelParseError::Layer2Read)?;
            let (objects, _) = parse_bytes(bytes, ObjectLayer::parse).map_err(LevelParseError::Layer2Read)?;
            Ok(Layer2Data::Objects(objects))
        }
    }

    fn parse_sh_and_sl(rom: &Rom, level_num: u32) -> Result<(SpriteHeader, SpriteLayer), LevelParseError> {
        const SPRITE_DATA: AddrSnes = AddrSnes(0x05EC00);

        let sprite_ptr_slice = SnesSlice::new(SPRITE_DATA + (2 * level_num), 2);
        let sh_addr = rom.parse_lorom(sprite_ptr_slice, le_u16).map_err(LevelParseError::SpriteAddressRead)?;
        let sh_addr = AddrSnes(sh_addr as _).with_bank(0x07);

        let sh_slice = SnesSlice::new(sh_addr, SPRITE_HEADER_SIZE);
        let sprite_header =
            rom.parse_lorom(sh_slice, SpriteHeader::read_from).map_err(LevelParseError::SpriteHeaderRead)?;

        let sprite_layer = {
            let bytes = rom.slice_from(sh_addr + 1).map_err(LevelParseError::SpriteRead)?;
            parse_bytes(bytes, SpriteLayer::parse).map_err(LevelParseError::SpriteRead)?.0
        };

        Ok((sprite_header, sprite_layer))
    }
}
