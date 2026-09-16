//! Real-ROM validation for the vanilla music track table
//! (`smwe_rom::music`, derived from SMWDisX `bank_05.asm` `LevelMusicTable`).
//!
//! Run with:
//! `ROM_PATH=~/workspace/smw-editor/smw.smc cargo test -p smwe-rom --test music_tracks -- --ignored`

use smwe_rom::{
    music::{format_music_track, music_track_name, MUSIC_TRACK_COUNT},
    SmwRom,
};

fn load_rom() -> SmwRom {
    let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH to a real SMW ROM");
    SmwRom::from_file(&rom_path).expect("parse ROM")
}

/// Every vanilla level's header music value must resolve through the track
/// table, and every one of the 8 vanilla tracks must actually be used by the
/// game (so no table entry is dead and no real level hits the fallback).
#[test]
#[ignore]
fn vanilla_track_table_covers_every_level() {
    let smw = load_rom();
    assert!(!smw.levels.is_empty());

    let mut seen = [false; 8];
    for (i, level) in smw.levels.iter().enumerate() {
        let m = level.primary_header.music();
        assert!(m < MUSIC_TRACK_COUNT, "level {i:#04X} has music value {m} outside the vanilla 0-7 range");
        seen[m as usize] = true;
        assert!(music_track_name(m).is_some(), "level {i:#04X}: track {m} has no name");
        // The UI label must never be empty for a real level.
        assert!(!format_music_track(m).is_empty());
    }
    assert!(seen.iter().all(|&s| s), "every vanilla track should be used by at least one level: {seen:?}");
}
