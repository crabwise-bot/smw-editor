//! Vanilla SMW music track names for the level-header music selector.
//!
//! Source: SMWDisX `bank_05.asm`, `LevelMusicTable` — read by `CODE_0584E3`
//! when the primary header is parsed. The 3-bit header value indexes this
//! table of SPC track IDs (the `!BGM_*` constants from SMWDisX
//! `constants.asm`):
//!
//! ```text
//! LevelMusicTable:
//!     db !BGM_OVERWORLD      ; 0
//!     db !BGM_UNDERGROUND    ; 1
//!     db !BGM_ATHLETIC       ; 2
//!     db !BGM_CASTLE         ; 3
//!     db !BGM_GHOSTHOUSE     ; 4
//!     db !BGM_UNDERWATER     ; 5
//!     db !BGM_BOSSFIGHT      ; 6
//!     db !BGM_BONUSGAME      ; 7
//! ```
//!
//! `!BGM_OVERWORLD = 2`, `!BGM_UNDERGROUND = 6`, `!BGM_ATHLETIC = 1`,
//! `!BGM_CASTLE = 8`, `!BGM_GHOSTHOUSE = 7`, `!BGM_UNDERWATER = 3`,
//! `!BGM_BOSSFIGHT = 5`, `!BGM_BONUSGAME = 18`.

/// Number of music tracks selectable by the vanilla 3-bit level-header field.
pub const MUSIC_TRACK_COUNT: u8 = 8;

/// SPC track IDs selected by each header value, in `LevelMusicTable` order.
const MUSIC_TRACK_SPC_IDS: [u8; 8] = [2, 6, 1, 8, 7, 3, 5, 18];

/// Display names for the 8 vanilla selectable tracks, in header-value order.
const MUSIC_TRACK_NAMES: [&str; 8] = [
    "Overworld",
    "Underground",
    "Athletic",
    "Castle",
    "Ghost House",
    "Underwater",
    "Boss Battle",
    "Bonus Game",
];

/// Display name for a level-header music value.
///
/// Returns `None` for values outside the vanilla 0-7 range (reachable in ROM
/// hacks that widen the field); callers should fall back to a raw-byte
/// rendering so the underlying value is never hidden.
pub fn music_track_name(track: u8) -> Option<&'static str> {
    MUSIC_TRACK_NAMES.get(track as usize).copied()
}

/// SPC track ID the vanilla game plays for a level-header music value.
///
/// Returns `None` outside the vanilla 0-7 range.
pub fn music_track_spc_id(track: u8) -> Option<u8> {
    MUSIC_TRACK_SPC_IDS.get(track as usize).copied()
}

/// Human-readable label for a level-header music value: `"3: Castle"`.
///
/// Values outside the vanilla range render as `"N: custom"` so a hacked ROM's
/// remapped track is still identifiable by its raw byte.
pub fn format_music_track(track: u8) -> String {
    match music_track_name(track) {
        Some(name) => format!("{track}: {name}"),
        None => format!("{track}: custom"),
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_table_matches_smwdisx_level_music_table() {
        // Order mirrors SMWDisX bank_05.asm `LevelMusicTable`.
        let expected = [
            "Overworld",
            "Underground",
            "Athletic",
            "Castle",
            "Ghost House",
            "Underwater",
            "Boss Battle",
            "Bonus Game",
        ];
        for (i, name) in expected.iter().enumerate() {
            assert_eq!(music_track_name(i as u8), Some(*name));
        }
    }

    #[test]
    fn spc_ids_match_smwdisx_bgm_constants() {
        // !BGM_OVERWORLD=2, !BGM_UNDERGROUND=6, !BGM_ATHLETIC=1, !BGM_CASTLE=8,
        // !BGM_GHOSTHOUSE=7, !BGM_UNDERWATER=3, !BGM_BOSSFIGHT=5,
        // !BGM_BONUSGAME=18 (SMWDisX constants.asm).
        let expected = [2u8, 6, 1, 8, 7, 3, 5, 18];
        for (i, id) in expected.iter().enumerate() {
            assert_eq!(music_track_spc_id(i as u8), Some(*id));
        }
    }

    #[test]
    fn out_of_range_values_fall_back_to_raw() {
        assert_eq!(music_track_name(8), None);
        assert_eq!(music_track_name(255), None);
        assert_eq!(music_track_spc_id(9), None);
        assert_eq!(format_music_track(3), "3: Castle");
        assert_eq!(format_music_track(42), "42: custom");
    }
}
