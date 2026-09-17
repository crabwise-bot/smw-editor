use std::convert::TryInto;

use nom::{bytes::complete::take, IResult};

use crate::{
    snes_utils::{addr::AddrSnes, rom::Rom, rom_slice::SnesSlice},
    RomError,
};

pub const PRIMARY_HEADER_SIZE: usize = 5;
pub const SECONDARY_HEADER_SIZE: usize = 4;
pub const SPRITE_HEADER_SIZE: usize = 1;

#[derive(Debug, Clone)]
pub struct PrimaryHeader(pub [u8; PRIMARY_HEADER_SIZE]);

#[derive(Debug, Clone)]
pub struct SecondaryHeader {
    pub bytes:      [u8; SECONDARY_HEADER_SIZE],
    /// LM 3.40+ Layer 2 scroll extension byte (SNES `$06FA00`, `SHCvvvvv`).
    /// `$FF` when LM never installed the table (vanilla ROM); see
    /// [`crate::level::scroll::Layer2ScrollExt`].
    pub scroll_ext: u8,
}

#[derive(Debug, Clone)]
pub struct SpriteHeader(pub u8);

impl PrimaryHeader {
    pub fn new(bytes: &[u8]) -> Self {
        Self(bytes.try_into().unwrap())
    }

    pub fn palette_bg(&self) -> u8 {
        // BBB----- -------- -------- -------- --------
        // palette_bg = BBB
        self.0[0] >> 5
    }

    pub fn level_length(&self) -> u8 {
        // ---LLLLL -------- -------- -------- --------
        // level_length = LLLLL
        self.0[0] & 0b11111
    }

    pub fn back_area_color(&self) -> u8 {
        // -------- CCC----- -------- -------- --------
        // back_area_color = CCC
        self.0[1] >> 5
    }

    pub fn level_mode(&self) -> u8 {
        // -------- ---MMMMM -------- -------- --------
        // level_mode = MMMMM
        self.0[1] & 0b11111
    }

    pub fn layer3_priority(&self) -> bool {
        // -------- -------- P------- -------- --------
        // layer3_priority = P
        (self.0[2] >> 7) != 0
    }

    pub fn music(&self) -> u8 {
        // -------- -------- -MMM---- -------- --------
        // music = MMM
        (self.0[2] >> 4) & 0b111
    }

    pub fn sprite_gfx(&self) -> u8 {
        // -------- -------- ----SSSS -------- --------
        // sprite_gfx = SSSS
        self.0[2] & 0b1111
    }

    pub fn timer(&self) -> u8 {
        // -------- -------- -------- TT------ --------
        // timer = TT
        self.0[3] >> 6
    }

    pub fn palette_sprite(&self) -> u8 {
        // -------- -------- -------- --PPP--- --------
        // palette_sprite = PPP
        (self.0[3] >> 3) & 0b111
    }

    pub fn palette_fg(&self) -> u8 {
        // -------- -------- -------- -----FFF --------
        // palette_fg = FFF
        self.0[3] & 0b111
    }

    pub fn item_memory(&self) -> u8 {
        // -------- -------- -------- -------- II------
        // item_memory = II
        self.0[4] >> 6
    }

    pub fn vertical_scroll(&self) -> u8 {
        // -------- -------- -------- -------- --VV----
        // vertical_scroll = VV
        (self.0[4] >> 4) & 0b11
    }

    pub fn fg_bg_gfx(&self) -> u8 {
        // -------- -------- -------- -------- ----GGGG
        // fg_bg_gfx = GGGG
        self.0[4] & 0b1111
    }
}

impl SecondaryHeader {
    pub fn read_from_rom(rom: &Rom, level_num: u32) -> Result<Self, RomError> {
        let mut bytes = [0; 4];
        let byte_table_addrs = [0x05F000, 0x05F200, 0x05F400, 0x05F600];
        for (byte, addr) in bytes.iter_mut().zip(byte_table_addrs) {
            let slice = SnesSlice::new(AddrSnes(addr), 0x200);
            let byte_table = rom.slice_lorom(slice)?;
            *byte = byte_table[level_num as usize];
        }
        // LM 3.40+ per-level Layer 2 scroll extension table ($06FA00, SHCvvvvv).
        // Vanilla ROMs have $FF here (table not installed).
        let scroll_ext = {
            let slice = SnesSlice::new(AddrSnes(0x06FA00), 0x200);
            let table = rom.slice_lorom(slice)?;
            table[level_num as usize]
        };
        Ok(Self { bytes, scroll_ext })
    }

    pub fn layer2_scroll(&self) -> u8 {
        // SSSS---- -------- -------- --------
        // layer2_scroll = SSSS
        (self.bytes[0] >> 4) & 0b1111
    }

    pub fn main_entrance_xy_pos(&self) -> (u8, u8) {
        // ----YYYY -----XXX -------- --------
        // main_entrance_xy_pos = (XXX, YYYY)
        let x = self.bytes[1] & 0b111;
        let y = self.bytes[0] & 0b1111;
        (x, y)
    }

    pub fn layer3(&self) -> u8 {
        // -------- LL------ -------- --------
        // layer3 = LL
        (self.bytes[1] >> 6) & 0b11
    }

    pub fn main_entrance_mario_action(&self) -> u8 {
        // -------- --AAA--- -------- --------
        // main_entrance_mario_action = AAA
        (self.bytes[1] >> 3) & 0b111
    }

    pub fn midway_entrance_screen(&self) -> u8 {
        // -------- -------- SSSS---- --------
        // midway_entrance_screen = SSSS
        (self.bytes[2] >> 4) & 0b1111
    }

    pub fn fg_initial_pos(&self) -> u8 {
        // -------- -------- ----FF-- --------
        // fg_initial_pos = FF
        (self.bytes[2] >> 2) & 0b11
    }

    pub fn bg_initial_pos(&self) -> u8 {
        // -------- -------- ------BB --------
        // bg_initial_pos = BB
        self.bytes[2] & 0b11
    }

    pub fn no_yoshi_level(&self) -> bool {
        // -------- -------- -------- Y-------
        // no_yoshi_level = Y
        (self.bytes[3] >> 7) != 0
    }

    pub fn unknown_vertical_pos_level(&self) -> bool {
        // -------- -------- -------- -U------
        // unknown_vertical_pos_level = U
        (self.bytes[3] & 0b01000000) != 0
    }

    pub fn vertical_level(&self) -> bool {
        // -------- -------- -------- --V-----
        // vertical_level = V
        (self.bytes[3] & 0b00100000) != 0
    }

    pub fn main_entrance_screen(&self) -> u8 {
        // -------- -------- -------- ---EEEEE
        // main_entrance_screen = EEEEE
        self.bytes[3] & 0b11111
    }
}

impl SpriteHeader {
    pub fn read_from(input: &[u8]) -> IResult<&[u8], Self> {
        let (input, bytes) = take(SPRITE_HEADER_SIZE)(input)?;
        Ok((input, Self(bytes[0])))
    }

    /// Build from a raw header byte.
    pub fn new(byte: u8) -> Self {
        Self(byte)
    }

    /// The raw header byte.
    pub fn as_byte(&self) -> u8 {
        self.0
    }

    /// Bit layout, verified against the vanilla game (`bank_05.asm`,
    /// level-load sprite setup):
    /// - `LDA [SpriteDataPtr] / AND #$3F -> $1692 (SpriteMemorySetting)`.
    ///   Bits 0-5 are the sprite memory setting; vanilla levels use
    ///   0x00-0x12 (the 19-entry `SpriteSlotMax/Start` tables, `bank_02.asm`).
    /// - `LDA [SpriteDataPtr] / AND #$C0 -> $190E (SpriteBuoyancy)`.
    ///   Bit 7 is buoyancy (`BEQ` test in `CODE_019211`); bit 6 disables
    ///   Layer 2 interaction (`BIT` + `BVS` test in the sprite/L2 routine).
    ///   Bit 5 is never set by any vanilla level (checked 2026-09-17:
    ///   bits ever set across all 512 levels are 0,1,2,3,4,6,7).
    pub fn sprite_buoyancy(&self) -> bool {
        // B-------
        // sprite_buoyancy = B
        (self.0 & 0b10000000) != 0
    }

    pub fn set_sprite_buoyancy(&mut self, on: bool) {
        if on {
            self.0 |= 0b10000000;
        } else {
            self.0 &= !0b10000000;
        }
    }

    pub fn disable_layer2_interaction(&self) -> bool {
        // -L------
        // disable_layer2_interaction = L
        (self.0 & 0b01000000) != 0
    }

    pub fn set_disable_layer2_interaction(&mut self, on: bool) {
        if on {
            self.0 |= 0b01000000;
        } else {
            self.0 &= !0b01000000;
        }
    }

    pub fn sprite_memory(&self) -> u8 {
        // --MMMMMM
        // sprite_memory = MMMMMM
        self.0 & 0b00111111
    }

    /// Set the sprite memory setting (bits 0-5, masked to `0x3F`);
    /// buoyancy / Layer 2 bits are preserved.
    pub fn set_sprite_memory(&mut self, value: u8) {
        self.0 = (self.0 & 0b11000000) | (value & 0b00111111);
    }
}
