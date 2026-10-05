//! Per-level music & time-limit bypass — Lunar Magic v1.70 parity.
//!
//! LM v1.70 (2003-09-24) added a "Change Music & Time Limit Settings" bypass
//! dialog that overrides a level's header music and time-limit settings; LM
//! v3.32 (2022-09-24) made it possible to type track numbers into the
//! music-bypass dropdown; LM v3.70 (2026-09-24) widened music track IDs to
//! 16 bits (up to 0x800, named).
//!
//! # Storage format (smw-editor native, documented)
//!
//! There is no vanilla ROM structure for this (LM implements it as an ASM
//! hack), so smw-editor stores the data in RATS-tagged free-space blocks. The
//! RATS tag is the standard `STAR` + size + ~size header LM itself uses, so
//! other tools' free-space scanners won't clobber the blocks.
//!
//! One RATS block for the per-level bypass table:
//!
//! ```text
//! "SMWMUSBP"   8 bytes magic
//! version       u8 (=1)
//! level_count   u16 LE
//! per level entry:
//!   level       u16 LE (0x000-0x1FF)
//!   music       u16 LE: 0xFFFF = no music bypass;
//!               otherwise the SPC track ID, 0x000-0x800 (LM v3.70 16-bit IDs)
//!   time        u16 LE: 0xFFFF = no time-limit bypass;
//!               otherwise seconds, 0-999 (0 = no time limit, like the header
//!               timer setting 0: vanilla `TimerTable` gives hundreds = 0, and
//!               the game skips the time-up check when the timer is all zero)
//! ```
//!
//! # Preview semantics
//!
//! The editor mirrors what LM's bypass does at level load: after the game's
//! normal header parse, the bypassed time limit is written to the emulated
//! `InGameTimerHundreds/Tens/Ones` (`$7E0F31-$7E0F33`, the exact addresses
//! `CODE_0584E3` writes via `TimerTable` — see SMWDisX `bank_05.asm`). The
//! bypassed music track is the level's *effective* music everywhere
//! smw-editor displays it (bypass dialog, Level Header override indicator).
//!
//! # In-game playback
//!
//! The editor authors, previews, and stores the data; making the real game
//! honor it still requires installing Lunar Magic's bypass ASM hack (LM does
//! this itself when you use its bypass tools). On a stock ROM the bypass is
//! inert in-game. The editor never plays audio, so the music half of the
//! bypass has no audible effect in smw-editor itself — it is recorded as the
//! level's effective music and shown wherever the editor displays music.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::exgfx::{find_rats_blocks, rats_size, write_rats_block, ExGfxError};

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Magic at the start of the music/time bypass RATS payload.
pub const MUSIC_BYPASS_MAGIC: &[u8; 8] = b"SMWMUSBP";
/// Payload format version.
pub const MUSIC_BYPASS_FORMAT_VERSION: u8 = 1;
/// Entry value meaning "no bypass for this field".
pub const MUSIC_BYPASS_NONE: u16 = 0xFFFF;
/// Highest music track ID the dialog accepts (LM v3.70 16-bit IDs).
pub const MUSIC_TRACK_MAX_ID: u16 = 0x800;
/// Highest time-limit value the dialog accepts, in seconds.
pub const TIME_LIMIT_MAX_SECONDS: u16 = 999;

/// Vanilla time limits selected by the 2-bit header timer field, in seconds.
/// SMWDisX `bank_05.asm` `TimerTable`: `db $00,$02,$03,$04` are the hundreds
/// digits (`CODE_0584E3` zeroes tens and ones), i.e. 0/200/300/400 seconds.
const VANILLA_TIMER_SECONDS: [u16; 4] = [0, 200, 300, 400];

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MusicBypassError {
    #[error("Music/time bypass data not found in ROM")]
    NotFound,
    #[error("Corrupt music/time bypass data: {0}")]
    Corrupt(String),
    #[error("Bad bypass level number {0:#05X}")]
    BadLevel(u16),
    #[error("Bad music track ID {0:#06X} (valid: 0x000-0x800)")]
    BadTrackId(u16),
    #[error("Bad time limit {0} seconds (valid: 0-999)")]
    BadTimeLimit(u16),
    #[error("No free space for {0} bytes of bypass data")]
    NoFreeSpace(usize),
}

// -------------------------------------------------------------------------------------------------
// Model
// -------------------------------------------------------------------------------------------------

/// Per-level music/time bypass: `None` = use the level header's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MusicBypass {
    /// SPC track ID override (0x000-0x800), or `None` for no music bypass.
    pub music:      Option<u16>,
    /// Time-limit override in seconds (0-999), or `None` for no time bypass.
    pub time_limit: Option<u16>,
}

impl MusicBypass {
    /// `true` when neither field is overridden (nothing to store).
    pub fn is_empty(&self) -> bool {
        self.music.is_none() && self.time_limit.is_none()
    }
}

/// Per-level music/time bypass table: level number → bypass.
#[derive(Clone, Debug, Default)]
pub struct MusicBypassData {
    pub levels: BTreeMap<u16, MusicBypass>,
}

impl MusicBypassData {
    /// Bypass for `level`, or `None` when the level has no bypass record.
    pub fn get(&self, level: u16) -> Option<MusicBypass> {
        self.levels.get(&level).copied()
    }

    /// Set (or clear with an empty bypass) the override for `level`.
    pub fn set(&mut self, level: u16, bypass: MusicBypass) -> Result<(), MusicBypassError> {
        if level >= 0x200 {
            return Err(MusicBypassError::BadLevel(level));
        }
        if let Some(t) = bypass.music {
            if t > MUSIC_TRACK_MAX_ID {
                return Err(MusicBypassError::BadTrackId(t));
            }
        }
        if let Some(s) = bypass.time_limit {
            if s > TIME_LIMIT_MAX_SECONDS {
                return Err(MusicBypassError::BadTimeLimit(s));
            }
        }
        if bypass.is_empty() {
            self.levels.remove(&level);
        } else {
            self.levels.insert(level, bypass);
        }
        Ok(())
    }

    /// Effective time limit in seconds for `level`: the bypass value, or the
    /// vanilla header-timer lookup when there is no time bypass.
    pub fn effective_time_seconds(&self, level: u16, header_timer: u8) -> u16 {
        self.levels
            .get(&level)
            .and_then(|b| b.time_limit)
            .unwrap_or_else(|| VANILLA_TIMER_SECONDS[usize::from(header_timer & 3)])
    }

    /// Effective music label for `level`: the bypass track ID, or the
    /// header's named track when there is no music bypass.
    pub fn effective_music_label(&self, level: u16, header_music: u8) -> String {
        match self.levels.get(&level).and_then(|b| b.music) {
            Some(track) => format_track_id(track),
            None => crate::music::format_music_track(header_music),
        }
    }

    fn decode_payload(payload: &[u8]) -> Result<Self, MusicBypassError> {
        let err = |m: &str| MusicBypassError::Corrupt(m.to_string());
        if payload.len() < 3 {
            return Err(err("bypass payload too short"));
        }
        if payload[0] != MUSIC_BYPASS_FORMAT_VERSION {
            return Err(err("unsupported bypass payload version"));
        }
        let count = u16::from_le_bytes([payload[1], payload[2]]) as usize;
        let want = 3 + count * 6;
        if payload.len() < want {
            return Err(err("bypass payload truncated"));
        }
        let mut levels = BTreeMap::new();
        let mut off = 3;
        for _ in 0..count {
            let level = u16::from_le_bytes([payload[off], payload[off + 1]]);
            let music = u16::from_le_bytes([payload[off + 2], payload[off + 3]]);
            let time = u16::from_le_bytes([payload[off + 4], payload[off + 5]]);
            off += 6;
            if level >= 0x200 {
                return Err(MusicBypassError::BadLevel(level));
            }
            if music != MUSIC_BYPASS_NONE && music > MUSIC_TRACK_MAX_ID {
                return Err(MusicBypassError::BadTrackId(music));
            }
            if time != MUSIC_BYPASS_NONE && time > TIME_LIMIT_MAX_SECONDS {
                return Err(MusicBypassError::BadTimeLimit(time));
            }
            let bypass = MusicBypass {
                music:      (music != MUSIC_BYPASS_NONE).then_some(music),
                time_limit: (time != MUSIC_BYPASS_NONE).then_some(time),
            };
            if !bypass.is_empty() {
                levels.insert(level, bypass);
            }
        }
        Ok(Self { levels })
    }

    fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(3 + self.levels.len() * 6);
        out.push(MUSIC_BYPASS_FORMAT_VERSION);
        out.extend_from_slice(&(self.levels.len() as u16).to_le_bytes());
        for (&level, bypass) in &self.levels {
            out.extend_from_slice(&level.to_le_bytes());
            out.extend_from_slice(&bypass.music.unwrap_or(MUSIC_BYPASS_NONE).to_le_bytes());
            out.extend_from_slice(&bypass.time_limit.unwrap_or(MUSIC_BYPASS_NONE).to_le_bytes());
        }
        out
    }

    /// Parse the bypass table from raw ROM bytes. Returns
    /// [`MusicBypassError::NotFound`] when no block exists yet (a fresh ROM).
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, MusicBypassError> {
        let tag =
            find_rats_blocks(rom_bytes, MUSIC_BYPASS_MAGIC).into_iter().next().ok_or(MusicBypassError::NotFound)?;
        let size = rats_size(rom_bytes, tag);
        // Payload follows the 8-byte magic (same layout as write_rats_block);
        // RATS size field = data_len - 1, hence the +1.
        Self::decode_payload(&rom_bytes[tag + 16..tag + 8 + size + 1])
    }

    /// Rewrite the bypass table in `rom_bytes` (old blocks erased first; no
    /// block is written when the table is empty).
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), MusicBypassError> {
        for tag in find_rats_blocks(rom_bytes, MUSIC_BYPASS_MAGIC) {
            let size = rats_size(rom_bytes, tag);
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }
        if self.levels.is_empty() {
            return Ok(());
        }
        let payload = self.encode_payload();
        write_rats_block(rom_bytes, MUSIC_BYPASS_MAGIC, &payload, header_offset).map_err(|e| match e {
            ExGfxError::NoFreeSpace(n) => MusicBypassError::NoFreeSpace(n),
            ExGfxError::TooLarge(n) => MusicBypassError::Corrupt(format!("bypass payload too large ({n} bytes)")),
            other => MusicBypassError::Corrupt(other.to_string()),
        })
    }
}

// -------------------------------------------------------------------------------------------------
// Track naming
// -------------------------------------------------------------------------------------------------

/// Named SPC track IDs for the bypass dropdown: the 8 vanilla header tracks
/// (SPC IDs from SMWDisX `LevelMusicTable`, names from PR #7) in track order,
/// plus LM v3.70's named 16-bit maximum.
///
/// LM v3.70 names track 0x800 in its dropdown; the exact label isn't quoted in
/// the public changelog/docs, so the entry is labeled by its ID and range role
/// rather than guessing LM's label.
pub fn bypass_named_tracks() -> Vec<(u16, &'static str)> {
    let mut out = Vec::with_capacity(9);
    for t in 0..crate::music::MUSIC_TRACK_COUNT {
        let id = crate::music::music_track_spc_id(t).expect("vanilla track table");
        let name = crate::music::music_track_name(t).expect("vanilla track table");
        out.push((u16::from(id), name));
    }
    out.push((MUSIC_TRACK_MAX_ID, "highest 16-bit track ID (LM v3.70)"));
    out
}

/// Human-readable label for a bypass track ID: `"2: Overworld"` for the named
/// vanilla tracks, `"800: highest 16-bit track ID (LM v3.70)"` for 0x800, and
/// `"3F: custom"` for anything else (ROM hacks with custom music).
pub fn format_track_id(track: u16) -> String {
    if let Some((_, name)) = bypass_named_tracks().into_iter().find(|(id, _)| *id == track) {
        return format!("{track:X}: {name}");
    }
    format!("{track:X}: custom")
}

// -------------------------------------------------------------------------------------------------
// Preview: apply the bypass to emulator WRAM
// -------------------------------------------------------------------------------------------------

/// WRAM offsets (from `$7E0000`) of the in-game timer the vanilla header
/// parse writes (`CODE_0584E3` via `TimerTable`; SMWDisX `rammap.asm`).
pub const WRAM_TIMER_HUNDREDS: usize = 0x0F31;
pub const WRAM_TIMER_TENS: usize = 0x0F32;
pub const WRAM_TIMER_ONES: usize = 0x0F33;

/// Split `seconds` (0-999) into BCD (hundreds, tens, ones) digits, the way
/// `CODE_0584E3` stores the timer in WRAM.
pub fn timer_bcd_digits(seconds: u16) -> (u8, u8, u8) {
    let seconds = seconds.min(999);
    ((seconds / 100) as u8, ((seconds / 10) % 10) as u8, (seconds % 10) as u8)
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_validates_track_and_time_ranges() {
        let mut data = MusicBypassData::default();
        assert!(data.set(0x105, MusicBypass { music: Some(MUSIC_TRACK_MAX_ID), time_limit: Some(999) }).is_ok());
        assert!(matches!(
            data.set(0x105, MusicBypass { music: Some(0x801), time_limit: None }),
            Err(MusicBypassError::BadTrackId(0x801))
        ));
        assert!(matches!(
            data.set(0x105, MusicBypass { music: None, time_limit: Some(1000) }),
            Err(MusicBypassError::BadTimeLimit(1000))
        ));
        assert!(matches!(data.set(0x200, MusicBypass::default()), Err(MusicBypassError::BadLevel(0x200))));
    }

    #[test]
    fn empty_bypass_clears_the_level_entry() {
        let mut data = MusicBypassData::default();
        data.set(0x105, MusicBypass { music: Some(2), time_limit: None }).unwrap();
        assert!(data.get(0x105).is_some());
        data.set(0x105, MusicBypass::default()).unwrap();
        assert!(data.get(0x105).is_none());
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut data = MusicBypassData::default();
        data.set(0x105, MusicBypass { music: Some(0x800), time_limit: Some(300) }).unwrap();
        data.set(0x10D, MusicBypass { music: None, time_limit: Some(0) }).unwrap();
        let payload = data.encode_payload();
        let back = MusicBypassData::decode_payload(&payload).unwrap();
        assert_eq!(back.get(0x105), Some(MusicBypass { music: Some(0x800), time_limit: Some(300) }));
        assert_eq!(back.get(0x10D), Some(MusicBypass { music: None, time_limit: Some(0) }));
    }

    #[test]
    fn decode_rejects_bad_version_and_truncation() {
        assert!(matches!(MusicBypassData::decode_payload(&[9, 0, 0]), Err(MusicBypassError::Corrupt(_))));
        assert!(matches!(MusicBypassData::decode_payload(&[1, 1, 0]), Err(MusicBypassError::Corrupt(_))));
        // Out-of-range track in stored data is corrupt, not silent.
        let mut data = MusicBypassData::default();
        data.set(0x105, MusicBypass { music: Some(0x800), time_limit: None }).unwrap();
        let mut payload = data.encode_payload();
        payload[5] = 0x01; // track high byte -> 0x801... actually music is at [5],[6]
        payload[6] = 0x08;
        assert!(matches!(MusicBypassData::decode_payload(&payload), Err(MusicBypassError::BadTrackId(0x801))));
    }

    #[test]
    fn effective_values_fall_back_to_header() {
        let data = MusicBypassData::default();
        // Header timer 2 -> vanilla 300 seconds.
        assert_eq!(data.effective_time_seconds(0x105, 2), 300);
        assert_eq!(data.effective_time_seconds(0x105, 0), 0);
        assert_eq!(data.effective_music_label(0x105, 3), "3: Castle");
    }

    #[test]
    fn effective_values_use_bypass_when_set() {
        let mut data = MusicBypassData::default();
        data.set(0x105, MusicBypass { music: Some(0x800), time_limit: Some(150) }).unwrap();
        assert_eq!(data.effective_time_seconds(0x105, 2), 150);
        assert!(data.effective_music_label(0x105, 3).starts_with("800:"));
    }

    #[test]
    fn track_id_labels() {
        assert_eq!(format_track_id(2), "2: Overworld");
        assert_eq!(format_track_id(0x12), "12: Bonus Game");
        assert_eq!(format_track_id(0x800), "800: highest 16-bit track ID (LM v3.70)");
        assert_eq!(format_track_id(0x3F), "3F: custom");
    }

    #[test]
    fn timer_bcd_digits_match_game_layout() {
        assert_eq!(timer_bcd_digits(0), (0, 0, 0));
        assert_eq!(timer_bcd_digits(300), (3, 0, 0));
        assert_eq!(timer_bcd_digits(215), (2, 1, 5));
        assert_eq!(timer_bcd_digits(999), (9, 9, 9));
        assert_eq!(timer_bcd_digits(1234), (9, 9, 9)); // clamped
    }
}
