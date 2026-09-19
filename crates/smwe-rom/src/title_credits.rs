//! Fixed-location title-screen and ending enemy-credit data.
//!
//! This covers the small, directly editable pieces of SMW's title/credits
//! subsystem:
//! - title screen submap immediate operand in `GM03LoadTitleScreen`
//! - title demo controller playback bytes in `TitleScreenInputSeq`
//! - ending enemy-name stripe images (`EnemyNameStripe00..0C`)
//!
//! The staff roll and credits scene scripts are separate systems in bank 0C and
//! are intentionally not modeled here.

use crate::{
    internal_header::{RegionCode, RomInternalHeader},
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::Rom,
    },
};

pub const TITLE_SUBMAP_OPERAND_SNES: AddrSnes = AddrSnes(0x0096CE);
pub const TITLE_INPUT_SEQ_SNES: AddrSnes = AddrSnes(0x009C1F);
pub const TITLE_INPUT_SEQ_MAX_SIZE: usize = 0x009C64 - 0x009C1F;
pub const TITLE_SCREEN_STRIPE_SNES: AddrSnes = AddrSnes(0x05B375);
pub const TITLE_SCREEN_STRIPE_END_SNES: AddrSnes = AddrSnes(0x05B7C9);
pub const TITLE_SCREEN_STRIPE_MAX_SIZE: usize = 0x05B7C9 - 0x05B375;

/// Player-select stripe (`PlayerSelectStripe` in SMWDisX `bank_05.asm`): the
/// "1 PLAYER GAME" / "2 PLAYER GAME" menu plus the blank clears around it.
/// Drawn by `LoadScrnImage` (stripe index `$12`) after the title logo stripe,
/// so it composes over the logo on the same Layer 3 tilemap. Fixed slot
/// `0x05B872..0x05B8C7` on the U ROM (85 bytes; the next stripe image starts
/// at `0x05B8C7`).
pub const PLAYER_SELECT_STRIPE_SNES: AddrSnes = AddrSnes(0x05B872);
pub const PLAYER_SELECT_STRIPE_END_SNES: AddrSnes = AddrSnes(0x05B8C7);
pub const PLAYER_SELECT_STRIPE_MAX_SIZE: usize = 0x05B8C7 - 0x05B872;

pub const ENEMY_NAME_COUNT: usize = 13;
pub const ENEMY_NAME_STRIPE_STARTS: [AddrSnes; ENEMY_NAME_COUNT] = [
    AddrSnes(0x0DF300),
    AddrSnes(0x0DF42D),
    AddrSnes(0x0DF572),
    AddrSnes(0x0DF66B),
    AddrSnes(0x0DF742),
    AddrSnes(0x0DF837),
    AddrSnes(0x0DF8FA),
    AddrSnes(0x0DF9CD),
    AddrSnes(0x0DFA98),
    AddrSnes(0x0DFB73),
    AddrSnes(0x0DFC58),
    AddrSnes(0x0DFCD5),
    AddrSnes(0x0DFD5C),
];
pub const ENEMY_NAME_STRIPE_END_SNES: AddrSnes = AddrSnes(0x0DFE5A);

pub const ENEMY_NAME_LABELS: [&str; ENEMY_NAME_COUNT] = [
    "Lakitu / Para-bombs",
    "Amazin' Flyin' Hammer Brother",
    "Sumo Brother",
    "Rex / Mega Mole / Banzai Bill",
    "Dino-Rhino / Dino-Torch / Koopas",
    "Spike Top / Swoopers / Buzzy Beetle / Blargg",
    "Blurps / Urchin / Porcu-Puffer / Torpedo Ted / Rip Van Fish",
    "Boo Buddies / Fishin' Boo / Big Boo / Eeries",
    "Lil Sparky / Bony Beetle / Dry Bones / Thwomps",
    "Grinder / Ball 'n' Chain / Fishbone",
    "Reznor",
    "Mechakoopas",
    "Koopalings / Bowser",
];

// -------------------------------------------------------------------------------------------------
// Regional layouts (Lunar Magic v1.30 parity: Japanese ROM support).
// -------------------------------------------------------------------------------------------------

/// Which regional fixed-slot layout the title/credits data uses.
///
/// Lunar Magic v1.30 (2001-09-24) added support for Japanese SMW ROMs. The
/// title screen and credits stripe images live at different fixed addresses
/// there; the stripe *format* (`LoadStripeImage`) is identical in every
/// region — only the slots move. Verified against SMWDisX `differences.txt`
/// and the `ver_is_japanese` branches of `bank_05.asm` / `bank_0D.asm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleCreditsRegion {
    /// U.S. layout. Also used for European and other non-Japanese ROMs:
    /// per `differences.txt` their title/credits slots sit at the same
    /// addresses as the U ROM (E0 `$05B375`, E1 `$05B375`, …).
    Us,
    /// Japanese layout.
    Japanese,
}

impl TitleCreditsRegion {
    /// Detect the layout from the ROM's internal header region code.
    pub fn of(header: &RomInternalHeader) -> Self {
        if matches!(header.region_code, RegionCode::Japan) {
            Self::Japanese
        } else {
            Self::Us
        }
    }

    /// Short human label for the UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Us => "U.S.",
            Self::Japanese => "Japanese",
        }
    }

    /// The fixed-slot layout for this region.
    pub fn layout(&self) -> &'static TitleCreditsLayout {
        match self {
            Self::Us => &LAYOUT_US,
            Self::Japanese => &LAYOUT_JP,
        }
    }
}

/// Fixed-slot addresses for the title/credits subsystem in one regional ROM
/// variant.
#[derive(Debug, Clone, Copy)]
pub struct TitleCreditsLayout {
    /// Immediate operand selecting the title screen's overworld submap
    /// (`LDA.B #…` in `GM03LoadTitleScreen`).
    pub title_submap_operand:     AddrSnes,
    /// `TitleScreenInputSeq`: the title demo controller playback.
    pub title_input_seq:          AddrSnes,
    /// Fixed slot size for the input sequence (`$FF`-terminated).
    pub title_input_seq_max:      usize,
    /// Title logo stripe image (`TitleScreenStripe`).
    pub title_stripe:             AddrSnes,
    /// Fixed slot size for the title stripe (`$FF`-terminated).
    pub title_stripe_max:         usize,
    /// Player-select menu stripe image (`PlayerSelectStripe`).
    pub player_select_stripe:     AddrSnes,
    /// Fixed slot size for the menu stripe (`$FF`-terminated).
    pub player_select_stripe_max: usize,
    /// Per-scene enemy-name stripe image starts (`EnemyNameStripe00..0C`).
    pub enemy_name_starts:        [AddrSnes; ENEMY_NAME_COUNT],
    /// End of the enemy-name stripe region (start of the next data block).
    pub enemy_name_end:           AddrSnes,
}

impl TitleCreditsLayout {
    /// Byte size of enemy-name scene `index`'s fixed slot.
    pub fn enemy_name_slot_size(&self, index: usize) -> usize {
        let start = self.enemy_name_starts[index].0 as usize;
        let end = if index + 1 < ENEMY_NAME_COUNT {
            self.enemy_name_starts[index + 1].0 as usize
        } else {
            self.enemy_name_end.0 as usize
        };
        end - start
    }
}

/// U.S. layout: the long-standing fixed addresses.
pub const LAYOUT_US: TitleCreditsLayout = TitleCreditsLayout {
    title_submap_operand:     TITLE_SUBMAP_OPERAND_SNES,
    title_input_seq:          TITLE_INPUT_SEQ_SNES,
    title_input_seq_max:      TITLE_INPUT_SEQ_MAX_SIZE,
    title_stripe:             TITLE_SCREEN_STRIPE_SNES,
    title_stripe_max:         TITLE_SCREEN_STRIPE_MAX_SIZE,
    player_select_stripe:     PLAYER_SELECT_STRIPE_SNES,
    player_select_stripe_max: PLAYER_SELECT_STRIPE_MAX_SIZE,
    enemy_name_starts:        ENEMY_NAME_STRIPE_STARTS,
    enemy_name_end:           ENEMY_NAME_STRIPE_END_SNES,
};

/// Japanese layout (Lunar Magic v1.30 parity).
///
/// Provenance (all cross-checked against the real U ROM via the shared
/// disassembly — the extracted U bytes are byte-identical to the ROM, so the
/// J branches describe the real J ROM):
/// - title stripe: J `$05AF2C` (`differences.txt`; "J version has different
///   graphics for title text"). The J `TitleScreenStripe` data is 893 bytes
///   including the `$FF` terminator and ends exactly where the J
///   `FileSelectStripe` starts (`$05B2A9`); there is no slack past the data.
/// - player-select stripe: J `$05B358` = `$05B2A9` + the 175-byte J
///   `FileSelectStripe` (the J version has no separate erase-file stripe).
///   The J `PlayerSelectStripe` data is 91 bytes ending at `$05B3B3`
///   (`ContinueSaveStripe`).
/// - enemy-name stripes: same region start `$0DF300`; per-scene J lengths
///   measured from the `ver_is_japanese` `EnemyNameStripe00..0C` data in
///   `bank_0D.asm` sum to exactly `$0DFCEB - $0DF300` = `$9EB`
///   (`differences.txt`: "J: Stripe images that make up the enemy names in
///   the credits … U +436"), so the scene boundaries below are exact. The J
///   ROM has no Special-World name-update stripes (U-only feature).
/// - title input sequence: J `$009BB4` (`differences.txt`); the slot runs to
///   the next J data block at `$009C42` (`$8E` bytes, `$FF`-terminated).
/// - title submap operand: J `$009663` = U `$0096CE` − `$6B`; the J/U code
///   delta is a flat `−$6B` across the whole `$00944A..$009765` span (no
///   differing blocks in between, per `differences.txt`).
pub const LAYOUT_JP: TitleCreditsLayout = TitleCreditsLayout {
    title_submap_operand:     AddrSnes(0x009663),
    title_input_seq:          AddrSnes(0x009BB4),
    title_input_seq_max:      0x009C42 - 0x009BB4,
    title_stripe:             AddrSnes(0x05AF2C),
    title_stripe_max:         893,
    player_select_stripe:     AddrSnes(0x05B358),
    player_select_stripe_max: 91,
    enemy_name_starts:        [
        AddrSnes(0x0DF300),
        AddrSnes(0x0DF427),
        AddrSnes(0x0DF4F4),
        AddrSnes(0x0DF5CF),
        AddrSnes(0x0DF6A8),
        AddrSnes(0x0DF79B),
        AddrSnes(0x0DF84A),
        AddrSnes(0x0DF913),
        AddrSnes(0x0DF9CC),
        AddrSnes(0x0DFA8F),
        AddrSnes(0x0DFB74),
        AddrSnes(0x0DFBF1),
        AddrSnes(0x0DFC76),
    ],
    enemy_name_end:           AddrSnes(0x0DFCEB),
};

#[derive(Debug, Clone)]
pub struct TitleDemoInput {
    pub buttons:  u8,
    pub duration: u8,
}

#[derive(Debug, Clone)]
pub struct TitleCreditsData {
    /// Regional fixed-slot layout, detected from the ROM's internal header
    /// at load time (Lunar Magic v1.30 parity: Japanese ROM support).
    pub region:               TitleCreditsRegion,
    pub title_submap:         u8,
    pub title_demo_inputs:    Vec<TitleDemoInput>,
    pub title_screen_stripe:  Vec<u8>,
    pub player_select_stripe: Vec<u8>,
    pub enemy_name_stripes:   Vec<Vec<u8>>,
}

impl TitleCreditsData {
    /// The fixed-slot layout for this ROM's region.
    pub fn layout(&self) -> &'static TitleCreditsLayout {
        self.region.layout()
    }

    /// UI label for enemy-name scene `index`. The U.S. scenes get their
    /// English enemy names; the Japanese credits text is katakana (no Latin
    /// decode is implemented), so J scenes are identified by number.
    pub fn enemy_name_label(&self, index: usize) -> String {
        match self.region {
            TitleCreditsRegion::Us => ENEMY_NAME_LABELS[index].to_string(),
            TitleCreditsRegion::Japanese => format!("Scene {index:02X}"),
        }
    }

    pub fn empty(region: TitleCreditsRegion) -> Self {
        Self {
            region,
            title_submap: 0,
            title_demo_inputs: Vec::new(),
            title_screen_stripe: vec![0xFF],
            player_select_stripe: vec![0xFF],
            enemy_name_stripes: vec![vec![0xFF]; ENEMY_NAME_COUNT],
        }
    }

    pub fn parse(rom: &Rom, region: TitleCreditsRegion) -> anyhow::Result<Self> {
        let layout = region.layout();
        let title_submap_pc = AddrPc::try_from_lorom(layout.title_submap_operand)?.as_index();
        let title_submap =
            *rom.0.get(title_submap_pc).ok_or_else(|| anyhow::anyhow!("title submap operand out of range"))?;

        let input_pc = AddrPc::try_from_lorom(layout.title_input_seq)?.as_index();
        let input_bytes = rom
            .0
            .get(input_pc..input_pc + layout.title_input_seq_max)
            .ok_or_else(|| anyhow::anyhow!("title input sequence out of range"))?;
        let mut title_demo_inputs = Vec::new();
        let mut i = 0;
        while i < input_bytes.len() {
            if input_bytes[i] == 0xFF {
                break;
            }
            if i + 1 >= input_bytes.len() {
                anyhow::bail!("title input sequence missing duration before end of fixed slot");
            }
            title_demo_inputs.push(TitleDemoInput { buttons: input_bytes[i], duration: input_bytes[i + 1] });
            i += 2;
        }

        let title_screen_stripe = read_terminated_slot(
            rom,
            layout.title_stripe,
            AddrSnes(layout.title_stripe.0 + layout.title_stripe_max as u32),
            "title screen stripe image",
        )?;

        let player_select_stripe = read_terminated_slot(
            rom,
            layout.player_select_stripe,
            AddrSnes(layout.player_select_stripe.0 + layout.player_select_stripe_max as u32),
            "player select stripe image",
        )?;

        let mut enemy_name_stripes = Vec::with_capacity(ENEMY_NAME_COUNT);
        for i in 0..ENEMY_NAME_COUNT {
            let end_snes =
                if i + 1 < ENEMY_NAME_COUNT { layout.enemy_name_starts[i + 1] } else { layout.enemy_name_end };
            enemy_name_stripes.push(read_terminated_slot(
                rom,
                layout.enemy_name_starts[i],
                end_snes,
                &format!("enemy name stripe {i}"),
            )?);
        }

        Ok(Self {
            region,
            title_submap,
            title_demo_inputs,
            title_screen_stripe,
            player_select_stripe,
            enemy_name_stripes,
        })
    }

    pub fn title_input_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let max = self.layout().title_input_seq_max;
        let mut bytes = Vec::with_capacity(self.title_demo_inputs.len() * 2 + 1);
        for input in &self.title_demo_inputs {
            bytes.push(input.buttons);
            bytes.push(input.duration);
        }
        bytes.push(0xFF);
        if bytes.len() > max {
            anyhow::bail!("Title demo input is {} bytes, but the vanilla fixed slot is only {max} bytes", bytes.len());
        }
        Ok(bytes)
    }

    pub fn enemy_name_slot_size(&self, index: usize) -> usize {
        self.layout().enemy_name_slot_size(index)
    }

    pub fn validate_enemy_name_stripe(&self, index: usize, bytes: &[u8]) -> anyhow::Result<()> {
        let slot_size = self.enemy_name_slot_size(index);
        if bytes.len() > slot_size {
            anyhow::bail!(
                "Credits enemy stripe {index:02X} is {} bytes, but its fixed vanilla slot is only {slot_size} bytes",
                bytes.len()
            );
        }
        if !bytes.ends_with(&[0xFF]) {
            anyhow::bail!("Credits enemy stripe {index:02X} must end with FF");
        }
        Ok(())
    }

    pub fn validate_title_screen_stripe(&self) -> anyhow::Result<()> {
        let max = self.layout().title_stripe_max;
        if self.title_screen_stripe.len() > max {
            anyhow::bail!(
                "Title screen stripe is {} bytes, but the vanilla fixed slot is only {max} bytes",
                self.title_screen_stripe.len()
            );
        }
        if !self.title_screen_stripe.ends_with(&[0xFF]) {
            anyhow::bail!("Title screen stripe must end with FF");
        }
        Ok(())
    }

    pub fn validate_player_select_stripe(&self) -> anyhow::Result<()> {
        let max = self.layout().player_select_stripe_max;
        if self.player_select_stripe.len() > max {
            anyhow::bail!(
                "Player select stripe is {} bytes, but the vanilla fixed slot is only {max} bytes",
                self.player_select_stripe.len()
            );
        }
        if !self.player_select_stripe.ends_with(&[0xFF]) {
            anyhow::bail!("Player select stripe must end with FF");
        }
        Ok(())
    }
}

fn read_terminated_slot(rom: &Rom, start: AddrSnes, end: AddrSnes, label: &str) -> anyhow::Result<Vec<u8>> {
    let start_pc = AddrPc::try_from_lorom(start)?.as_index();
    let end_pc = AddrPc::try_from_lorom(end)?.as_index();
    let slot = rom.0.get(start_pc..end_pc).ok_or_else(|| anyhow::anyhow!("{label} out of range"))?;
    let end = slot.iter().position(|&b| b == 0xFF).map(|p| p + 1).unwrap_or(slot.len());
    Ok(slot[..end].to_vec())
}

pub fn decode_credit_tile(byte: u8) -> char {
    match byte {
        0x0A..=0x23 => (b'A' + (byte - 0x0A)) as char,
        0x24 => '-',
        0x27 => '.',
        0x85 | 0x86 => '\'',
        0xFC => ' ',
        _ => '?',
    }
}

pub fn summarize_enemy_name_stripe(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == 0xFF {
            break;
        }
        let len = bytes[i + 3] as usize + 1;
        let data_start = i + 4;
        let data_end = data_start + len;
        if data_end > bytes.len() {
            break;
        }

        let text_like = bytes[data_start..data_end]
            .chunks_exact(2)
            .filter(|pair| matches!(pair[0], 0x0A..=0x27 | 0x85 | 0x86 | 0xFC) && matches!(pair[1], 0x00 | 0x38 | 0x78))
            .count();
        if text_like >= 2 {
            if !out.is_empty() {
                out.push_str(" / ");
            }
            for pair in bytes[data_start..data_end].chunks_exact(2) {
                out.push(decode_credit_tile(pair[0]));
            }
        }
        i = data_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_input_bytes_include_terminator() {
        let data = TitleCreditsData {
            region:               TitleCreditsRegion::Us,
            title_submap:         0,
            title_demo_inputs:    vec![TitleDemoInput { buttons: 0x41, duration: 0x0F }],
            title_screen_stripe:  vec![0xFF],
            player_select_stripe: vec![0xFF],
            enemy_name_stripes:   vec![Vec::new(); ENEMY_NAME_COUNT],
        };
        assert_eq!(data.title_input_bytes().unwrap(), vec![0x41, 0x0F, 0xFF]);
    }

    #[test]
    fn enemy_name_slots_are_positive() {
        for layout in [TitleCreditsRegion::Us.layout(), TitleCreditsRegion::Japanese.layout()] {
            for i in 0..ENEMY_NAME_COUNT {
                assert!(layout.enemy_name_slot_size(i) > 0);
            }
        }
    }

    #[test]
    fn empty_data_has_saveable_enemy_stripes() {
        for region in [TitleCreditsRegion::Us, TitleCreditsRegion::Japanese] {
            let data = TitleCreditsData::empty(region);
            assert_eq!(data.title_screen_stripe, &[0xFF]);
            data.validate_title_screen_stripe().unwrap();
            assert_eq!(data.enemy_name_stripes.len(), ENEMY_NAME_COUNT);
            for (i, stripe) in data.enemy_name_stripes.iter().enumerate() {
                assert_eq!(stripe, &[0xFF]);
                data.validate_enemy_name_stripe(i, stripe).unwrap();
            }
        }
    }

    #[test]
    fn enemy_name_validation_rejects_missing_terminator() {
        let data = TitleCreditsData::empty(TitleCreditsRegion::Us);
        let err = data.validate_enemy_name_stripe(0, &[0x20, 0x00]).unwrap_err();
        assert!(err.to_string().contains("must end with FF"));
    }

    #[test]
    fn us_layout_matches_legacy_constants() {
        // The U.S. layout is exactly the long-standing fixed addresses.
        let l = TitleCreditsRegion::Us.layout();
        assert_eq!(l.title_submap_operand, TITLE_SUBMAP_OPERAND_SNES);
        assert_eq!(l.title_input_seq, TITLE_INPUT_SEQ_SNES);
        assert_eq!(l.title_input_seq_max, TITLE_INPUT_SEQ_MAX_SIZE);
        assert_eq!(l.title_stripe, TITLE_SCREEN_STRIPE_SNES);
        assert_eq!(l.title_stripe_max, TITLE_SCREEN_STRIPE_MAX_SIZE);
        assert_eq!(l.player_select_stripe, PLAYER_SELECT_STRIPE_SNES);
        assert_eq!(l.player_select_stripe_max, PLAYER_SELECT_STRIPE_MAX_SIZE);
        assert_eq!(l.enemy_name_starts, ENEMY_NAME_STRIPE_STARTS);
        assert_eq!(l.enemy_name_end, ENEMY_NAME_STRIPE_END_SNES);
    }

    #[test]
    fn japanese_layout_enemy_region_is_contiguous() {
        // The J enemy-name scene boundaries (measured from the J disassembly
        // data) must tile the region start..end with no gaps or overlaps.
        let l = TitleCreditsRegion::Japanese.layout();
        assert_eq!(l.enemy_name_starts[0], AddrSnes(0x0DF300));
        for i in 0..ENEMY_NAME_COUNT - 1 {
            assert!(l.enemy_name_starts[i] < l.enemy_name_starts[i + 1]);
        }
        assert!(l.enemy_name_starts[ENEMY_NAME_COUNT - 1] < l.enemy_name_end);
        // Total J region size matches differences.txt ($0DFCEB - $0DF300).
        assert_eq!(l.enemy_name_end.0 - l.enemy_name_starts[0].0, 0x9EB);
    }

    #[test]
    #[ignore]
    fn real_rom_uses_us_region_layout() {
        // Justin's ROM is a U.S. dump; the region plumbing must select the
        // U.S. layout for it.
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH to a real SMW ROM to run this test");
        let smw_rom = crate::SmwRom::from_file(&rom_path).expect("parse ROM");
        assert_eq!(TitleCreditsRegion::of(&smw_rom.internal_header), TitleCreditsRegion::Us);
        assert_eq!(smw_rom.title_credits.region, TitleCreditsRegion::Us);
        assert_eq!(smw_rom.title_credits.layout().title_stripe, TITLE_SCREEN_STRIPE_SNES);
    }

    #[test]
    fn region_detection_maps_japan_only() {
        assert_eq!(TitleCreditsRegion::Us.label(), "U.S.");
        assert_eq!(TitleCreditsRegion::Japanese.label(), "Japanese");
        assert_eq!(TitleCreditsRegion::Japanese.layout().title_stripe, AddrSnes(0x05AF2C));
        // Every non-Japanese region code uses the U.S. layout (their
        // title/credits slots match the U addresses per differences.txt).
        assert_eq!(TitleCreditsRegion::Us.layout().title_stripe, TITLE_SCREEN_STRIPE_SNES);
    }

    /// Synthetic 512 KiB ROM with a valid LoROM internal header; `region_byte`
    /// is the header region byte (0x00 = Japan, 0x01 = North America). Places
    /// a synthetic title stripe in the *Japanese* title slot, a different
    /// decoy stripe in the *U.S.* title slot, empty (`$FF`) enemy scenes at
    /// the J scene starts, an empty input sequence at the J input slot, and
    /// submap 3 at the J submap operand. No real ROM data is involved.
    fn synthetic_jp_rom(region_byte: u8) -> Rom {
        use crate::internal_header::sizes;
        let mut buf = vec![0u8; 0x80000];
        let hb = 0x7FC0usize;
        let mut name_field = [b' '; sizes::INTERNAL_ROM_NAME];
        let name = b"SUPER MARIOWORLD";
        name_field[..name.len()].copy_from_slice(name);
        buf[hb..hb + sizes::INTERNAL_ROM_NAME].copy_from_slice(&name_field);
        buf[hb + 0x15] = 0x20; // LoROM map mode
        buf[hb + 0x19] = region_byte;
        let checksum: u16 = 0x1234;
        buf[hb + 0x1C..hb + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        buf[hb + 0x1E..hb + 0x20].copy_from_slice(&checksum.to_le_bytes());

        let put = |buf: &mut Vec<u8>, snes: u32, bytes: &[u8]| {
            let pc = AddrPc::try_from_lorom(AddrSnes(snes)).unwrap().as_index();
            buf[pc..pc + bytes.len()].copy_from_slice(bytes);
        };
        // J title stripe: one horizontal command writing tile $1234 at (0,0).
        put(&mut buf, 0x05AF2C, &[0x50, 0x00, 0x00, 0x01, 0x34, 0x12, 0xFF]);
        // Decoy at the U.S. title slot: must NOT be picked up on a J parse.
        put(&mut buf, 0x05B375, &[0x50, 0x00, 0x00, 0x01, 0x78, 0x56, 0xFF]);
        // Empty enemy scenes at the J scene starts.
        for s in TitleCreditsRegion::Japanese.layout().enemy_name_starts {
            put(&mut buf, s.0, &[0xFF]);
        }
        // Empty title input sequence + submap 3 at the J addresses.
        put(&mut buf, 0x009BB4, &[0xFF]);
        put(&mut buf, 0x009663, &[0x03]);
        Rom::new(buf).unwrap()
    }

    #[test]
    fn detects_region_from_header() {
        use crate::internal_header::RomInternalHeader;
        let rom = synthetic_jp_rom(0x00);
        let header = RomInternalHeader::parse(&rom).unwrap();
        assert_eq!(TitleCreditsRegion::of(&header), TitleCreditsRegion::Japanese);
        let rom = synthetic_jp_rom(0x01);
        let header = RomInternalHeader::parse(&rom).unwrap();
        assert_eq!(TitleCreditsRegion::of(&header), TitleCreditsRegion::Us);
        // Europe (and every other non-Japanese region) uses the U.S. layout.
        let rom = synthetic_jp_rom(0x02);
        let header = RomInternalHeader::parse(&rom).unwrap();
        assert_eq!(TitleCreditsRegion::of(&header), TitleCreditsRegion::Us);
    }

    #[test]
    fn japanese_parse_uses_japanese_slots() {
        use crate::title_stripe::TitleTileGrid;
        let rom = synthetic_jp_rom(0x00);
        let data = TitleCreditsData::parse(&rom, TitleCreditsRegion::Japanese).unwrap();
        assert_eq!(data.region, TitleCreditsRegion::Japanese);
        // The title stripe came from the J slot, not the U.S. decoy.
        assert_eq!(data.title_screen_stripe, vec![0x50, 0x00, 0x00, 0x01, 0x34, 0x12, 0xFF]);
        // …and it decodes through the shared stripe codec.
        let grid = TitleTileGrid::from_stripe(&data.title_screen_stripe).unwrap();
        assert_eq!(grid.cells[0][0], 0x1234);
        // Enemy scenes parsed at the J boundaries.
        assert_eq!(data.enemy_name_stripes.len(), ENEMY_NAME_COUNT);
        for stripe in &data.enemy_name_stripes {
            assert_eq!(stripe, &vec![0xFF]);
        }
        // Input sequence + submap operand from the J addresses.
        assert!(data.title_demo_inputs.is_empty());
        assert_eq!(data.title_submap, 0x03);
        // Slot sizes come from the J layout …
        assert_eq!(data.enemy_name_slot_size(0), 0x0DF427 - 0x0DF300);
        assert_eq!(data.layout().title_stripe_max, 893);
        assert_eq!(data.layout().player_select_stripe_max, 91);
        // … and validation enforces the J budgets (893, not the U.S. 1108).
        let mut over = data.clone();
        over.title_screen_stripe = vec![0xFF; 894];
        assert!(over.validate_title_screen_stripe().is_err());
        let mut under = data.clone();
        under.title_screen_stripe = vec![0xFF; 893];
        under.validate_title_screen_stripe().unwrap();
        // J scenes get numeric labels (the J credits text is katakana; no
        // Latin decode is implemented).
        assert_eq!(data.enemy_name_label(0), "Scene 00");
        let us = TitleCreditsData::empty(TitleCreditsRegion::Us);
        assert_eq!(us.enemy_name_label(0), "Lakitu / Para-bombs");
    }
}
