//! Overworld submap music selection (Lunar Magic v1.30 parity).
//!
//! LM v1.30 "added the ability to change the music of the submaps on the
//! overworld". The vanilla game keeps two parallel 7-byte tables, one entry
//! per submap (0..=6, the same indexing as
//! [`SUBMAP_NAMES`][crate::overworld::SUBMAP_NAMES]):
//!
//! - `OverworldMusic` at SNES `$048D8A` — read at overworld init by
//!   `CODE_048E38` (`LDX.W PlayerTurnLvl` / `LDA.W OWPlayerSubmap,X` /
//!   `LDA.W OverworldMusic,X` / `STA.W SPCIO2`), SMWDisX `bank_04.asm`.
//! - `OverworldMusic2` at SNES `$04DBC8` — read on submap swap by
//!   `CODE_04DBCF` (`LDA.W OWPlayerSubmap,Y` / `LDA.L OverworldMusic2,X` /
//!   `STA.W SPCIO2`), SMWDisX `bank_04.asm`.
//!
//! Both tables carry the same 7 bytes on a vanilla ROM (verified
//! byte-for-byte against a real ROM: `[2, 3, 4, 6, 7, 9, 5]`), and both must
//! be written in sync — otherwise the music would change when the player
//! warps between submaps but not on overworld init (or vice versa).
//!
//! Each byte is an SPC music track ID (the `!BGM_*` constants from SMWDisX
//! `constants.asm`). The overworld-appropriate tracks are 2–10:
//!
//! ```text
//!  2 = Overworld (the main-map theme; also !BGM_DONUTPLAINS)
//!  3 = Yoshi's Island
//!  4 = Vanilla Dome
//!  5 = Star World
//!  6 = Forest of Illusion
//!  7 = Valley of Bowser
//!  8 = Valley Opens
//!  9 = Special World
//! 10 = Credits (Yoshi's House)
//! ```
//!
//! Editing is in place — two fixed 7-byte writes — so no relocation patch is
//! needed and untouched ROMs stay byte-identical.

use crate::snes_utils::addr::{AddrPc, AddrSnes};

/// SNES address of the 7-byte `OverworldMusic` table (read at overworld init).
pub const OVERWORLD_MUSIC_SNES: AddrSnes = AddrSnes(0x048D8A);
/// SNES address of the 7-byte `OverworldMusic2` table (read on submap swap).
pub const OVERWORLD_MUSIC2_SNES: AddrSnes = AddrSnes(0x04DBC8);
/// One byte per submap, matching [`SUBMAP_COUNT`][crate::overworld::SUBMAP_COUNT].
pub const SUBMAP_MUSIC_LEN: usize = 7;

/// The overworld music tracks LM v1.30 offers, as `(SPC track ID, name)`
/// pairs. Names transcribe SMWDisX `constants.asm` (`!BGM_*`); track 2 is
/// shown as "Overworld" to match the level-header music picker's naming in
/// [`crate::music`] (`!BGM_OVERWORLD = 2`, the main-map theme).
pub const SUBMAP_MUSIC_TRACKS: [(u8, &str); 9] = [
    (2, "Overworld"),
    (3, "Yoshi's Island"),
    (4, "Vanilla Dome"),
    (5, "Star World"),
    (6, "Forest of Illusion"),
    (7, "Valley of Bowser"),
    (8, "Valley Opens"),
    (9, "Special World"),
    (10, "Credits"),
];

/// Display name for an overworld SPC track ID. Returns `None` for IDs outside
/// the overworld-appropriate 2–10 range.
pub fn submap_music_track_name(track: u8) -> Option<&'static str> {
    SUBMAP_MUSIC_TRACKS.iter().find(|&&(id, _)| id == track).map(|&(_, name)| name)
}

/// Human-readable label for a track ID: `"3: Yoshi's Island"`. IDs outside
/// 2–10 render as `"N: custom"` so a hacked ROM's remapped track is still
/// identifiable by its raw byte.
pub fn format_submap_music_track(track: u8) -> String {
    match submap_music_track_name(track) {
        Some(name) => format!("{track}: {name}"),
        None => format!("{track}: custom"),
    }
}

/// Per-submap overworld music: one SPC track ID per submap 0..=6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmapMusic {
    /// `tracks[submap]` — the SPC music track the game plays on that submap.
    pub tracks: [u8; SUBMAP_MUSIC_LEN],
}

impl SubmapMusic {
    /// The exact vanilla bytes of both tables, transcribed from SMWDisX
    /// `bank_04.asm` (`OverworldMusic` / `OverworldMusic2`) and verified
    /// byte-for-byte against a real ROM.
    pub const VANILLA: [u8; SUBMAP_MUSIC_LEN] = [2, 3, 4, 6, 7, 9, 5];

    fn table_pc(addr: AddrSnes, what: &str, header_offset: usize) -> anyhow::Result<usize> {
        let pc = AddrPc::try_from_lorom(addr).map_err(|e| anyhow::anyhow!("{what} addr conversion: {e}"))?.0 as usize
            + header_offset;
        Ok(pc)
    }

    /// Parse the `OverworldMusic` table (`$048D8A`) from ROM bytes
    /// (`header_offset` = `0x200` if the ROM has an SMC header, else `0`).
    ///
    /// Only the init-time table is parsed: the submap-swap table
    /// (`$04DBC8`) is a mirror of it on every vanilla ROM, and saves always
    /// rewrite both in sync.
    pub fn parse(rom: &[u8], header_offset: usize) -> anyhow::Result<Self> {
        let pc = Self::table_pc(OVERWORLD_MUSIC_SNES, "submap music", header_offset)?;
        let bytes = rom
            .get(pc..pc + SUBMAP_MUSIC_LEN)
            .ok_or_else(|| anyhow::anyhow!("submap music table extends past end of ROM"))?;
        let mut tracks = [0u8; SUBMAP_MUSIC_LEN];
        tracks.copy_from_slice(bytes);
        Ok(Self { tracks })
    }

    /// Parse both tables and report whether they agree. Vanilla ROMs (and
    /// Lunar Magic) keep the two mirrors identical; a disagreement means a
    /// foreign tool edited one without the other.
    pub fn tables_in_sync(rom: &[u8], header_offset: usize) -> anyhow::Result<bool> {
        let pc1 = Self::table_pc(OVERWORLD_MUSIC_SNES, "submap music", header_offset)?;
        let pc2 = Self::table_pc(OVERWORLD_MUSIC2_SNES, "submap music 2", header_offset)?;
        let a = rom
            .get(pc1..pc1 + SUBMAP_MUSIC_LEN)
            .ok_or_else(|| anyhow::anyhow!("submap music table extends past end of ROM"))?;
        let b = rom
            .get(pc2..pc2 + SUBMAP_MUSIC_LEN)
            .ok_or_else(|| anyhow::anyhow!("submap music table 2 extends past end of ROM"))?;
        Ok(a == b)
    }

    /// Encode the 7 track bytes.
    pub fn encode(&self) -> [u8; SUBMAP_MUSIC_LEN] {
        self.tracks
    }

    /// Set the music for one submap. Refuses submap indices >= 7 and track
    /// IDs outside the overworld-appropriate 2–10 range (the dialog only
    /// offers those; other bytes are left to hex editors, matching how LM's
    /// dialog works).
    pub fn set(&mut self, submap: usize, track: u8) -> bool {
        if submap >= SUBMAP_MUSIC_LEN || submap_music_track_name(track).is_none() {
            return false;
        }
        self.tracks[submap] = track;
        true
    }

    /// Write both tables back into `rom_bytes` in place, in sync.
    pub fn apply_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> anyhow::Result<()> {
        for (addr, what) in [(OVERWORLD_MUSIC_SNES, "submap music"), (OVERWORLD_MUSIC2_SNES, "submap music 2")] {
            let pc = Self::table_pc(addr, what, header_offset)?;
            let dst = rom_bytes
                .get_mut(pc..pc + SUBMAP_MUSIC_LEN)
                .ok_or_else(|| anyhow::anyhow!("{what} write range out of bounds"))?;
            dst.copy_from_slice(&self.encode());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_names_cover_2_through_10() {
        let expected = [
            "Overworld",
            "Yoshi's Island",
            "Vanilla Dome",
            "Star World",
            "Forest of Illusion",
            "Valley of Bowser",
            "Valley Opens",
            "Special World",
            "Credits",
        ];
        for (i, name) in expected.iter().enumerate() {
            assert_eq!(submap_music_track_name(2 + i as u8), Some(*name));
        }
        assert_eq!(submap_music_track_name(0), None);
        assert_eq!(submap_music_track_name(1), None);
        assert_eq!(submap_music_track_name(11), None);
        assert_eq!(submap_music_track_name(255), None);
    }

    #[test]
    fn format_labels_match_music_picker_convention() {
        assert_eq!(format_submap_music_track(2), "2: Overworld");
        assert_eq!(format_submap_music_track(9), "9: Special World");
        assert_eq!(format_submap_music_track(42), "42: custom");
    }

    #[test]
    fn vanilla_transcription_matches_smwdisx() {
        // OverworldMusic / OverworldMusic2 in SMWDisX bank_04.asm:
        // DONUTPLAINS, YOSHISISLAND, VANILLADOME, FORESTOFILLUSION,
        // VALLEYOFBOWSER, SPECIALWORLD, STARWORLD.
        assert_eq!(SubmapMusic::VANILLA, [2, 3, 4, 6, 7, 9, 5]);
    }

    #[test]
    fn set_validates_submap_and_track() {
        let mut m = SubmapMusic { tracks: SubmapMusic::VANILLA };
        assert!(m.set(0, 5));
        assert_eq!(m.tracks[0], 5);
        // Out-of-range submap refused.
        assert!(!m.set(7, 5));
        assert!(!m.set(usize::MAX, 5));
        // Non-overworld track refused.
        assert!(!m.set(0, 1));
        assert!(!m.set(0, 18));
        assert_eq!(m.tracks[0], 5); // unchanged by the refusals
    }

    #[test]
    fn apply_writes_both_tables_in_place() {
        let pc1 = AddrPc::try_from_lorom(OVERWORLD_MUSIC_SNES).unwrap().0 as usize;
        let pc2 = AddrPc::try_from_lorom(OVERWORLD_MUSIC2_SNES).unwrap().0 as usize;
        let mut rom = vec![0xAAu8; pc2 + SUBMAP_MUSIC_LEN + 4];
        rom[pc1..pc1 + SUBMAP_MUSIC_LEN].copy_from_slice(&SubmapMusic::VANILLA);
        rom[pc2..pc2 + SUBMAP_MUSIC_LEN].copy_from_slice(&SubmapMusic::VANILLA);

        let mut m = SubmapMusic::parse(&rom, 0).unwrap();
        assert_eq!(m.tracks, SubmapMusic::VANILLA);
        assert!(SubmapMusic::tables_in_sync(&rom, 0).unwrap());

        assert!(m.set(6, 2)); // Star World plays the Overworld theme
        m.apply_to_rom(&mut rom, 0).unwrap();

        let again = SubmapMusic::parse(&rom, 0).unwrap();
        assert_eq!(again.tracks[6], 2);
        // Both tables got the same bytes.
        assert_eq!(&rom[pc1..pc1 + SUBMAP_MUSIC_LEN], &rom[pc2..pc2 + SUBMAP_MUSIC_LEN]);
        assert!(SubmapMusic::tables_in_sync(&rom, 0).unwrap());
        // Neighbors untouched.
        assert_eq!(rom[pc1 - 1], 0xAA);
        assert_eq!(rom[pc2 + SUBMAP_MUSIC_LEN], 0xAA);
    }

    #[test]
    fn tables_in_sync_detects_mirror_mismatch() {
        let pc1 = AddrPc::try_from_lorom(OVERWORLD_MUSIC_SNES).unwrap().0 as usize;
        let pc2 = AddrPc::try_from_lorom(OVERWORLD_MUSIC2_SNES).unwrap().0 as usize;
        let mut rom = vec![0u8; pc2 + SUBMAP_MUSIC_LEN + 4];
        rom[pc1..pc1 + SUBMAP_MUSIC_LEN].copy_from_slice(&SubmapMusic::VANILLA);
        rom[pc2..pc2 + SUBMAP_MUSIC_LEN].copy_from_slice(&SubmapMusic::VANILLA);
        rom[pc2 + 3] = 2; // foreign tool touched only the swap table
        assert!(!SubmapMusic::tables_in_sync(&rom, 0).unwrap());
    }

    /// Parse the real ROM: both tables must be byte-identical and match the
    /// disassembly transcription. Run with
    /// `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib -- --ignored
    /// real_rom_submap_music`.
    #[test]
    #[ignore]
    fn real_rom_submap_music() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let raw = std::fs::read(rom_path).expect("read ROM");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

        let m = SubmapMusic::parse(&rom_bytes, 0).expect("submap music parse");
        assert_eq!(m.tracks, SubmapMusic::VANILLA, "real ROM differs from the disassembly transcription");
        assert!(SubmapMusic::tables_in_sync(&rom_bytes, 0).unwrap(), "the two vanilla tables should be mirrors");

        // Byte-identical write-back on an untouched table.
        let mut copy = rom_bytes.clone();
        m.apply_to_rom(&mut copy, 0).unwrap();
        assert_eq!(copy, rom_bytes);
    }
}
