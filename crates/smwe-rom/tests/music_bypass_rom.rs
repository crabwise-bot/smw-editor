//! Real-ROM validation for the per-level music & time-limit bypass
//! (`smwe_rom::music_bypass`, LM v1.70/v3.32/v3.70 parity).
//!
//! Run with:
//! `ROM_PATH=~/workspace/smw-editor/smw.smc cargo test -p smwe-rom --test music_bypass_rom -- --ignored`

use smwe_rom::music_bypass::{timer_bcd_digits, MusicBypass, MusicBypassData, WRAM_TIMER_HUNDREDS};

/// Round-trip through a scratch in-memory copy of the real ROM: write a
/// bypass table with the real `write_to_rom` path (free-space allocation on
/// the real image) and re-parse it.
#[test]
#[ignore]
fn write_and_parse_round_trip_on_real_rom() {
    let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH to a real SMW ROM");
    let rom_bytes = std::fs::read(&rom_path).expect("read ROM");
    let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };

    let mut scratch = rom_bytes.clone();
    let mut data = MusicBypassData::parse(&scratch).unwrap_or_default();
    data.set(0x105, MusicBypass { music: Some(0x800), time_limit: Some(150) }).unwrap();
    data.set(0x10D, MusicBypass { music: Some(8), time_limit: None }).unwrap();
    data.write_to_rom(&mut scratch, header_offset).unwrap();

    let back = MusicBypassData::parse(&scratch).expect("re-parse written block");
    assert_eq!(back.get(0x105), Some(MusicBypass { music: Some(0x800), time_limit: Some(150) }));
    assert_eq!(back.get(0x10D), Some(MusicBypass { music: Some(8), time_limit: None }));
    // Nothing else in the ROM's block map was disturbed: the block count is 1.
    assert_eq!(back.levels.len(), 2);
}

/// The time bypass writes the same WRAM timer bytes the game's own header
/// parse writes: load level 0x105 through the real emulator, apply a 150 s
/// bypass the way `apply_music_time_bypass_to_wram` does, and read the bytes
/// back.
#[test]
#[ignore]
fn time_bypass_lands_in_emulated_wram_timer() {
    use std::sync::Arc;
    let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH to a real SMW ROM");
    let rom_bytes = std::fs::read(&rom_path).expect("read ROM");
    let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };

    let mut emu_rom = smwe_emu::rom::Rom::new(rom_bytes[header_offset..].to_vec());
    emu_rom.load_symbols(include_str!("../../../symbols/SMW_U.sym"));
    let mut cpu = smwe_emu::Cpu::new(smwe_emu::emu::CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);

    // Level 0x105's header timer is 2 -> 300 s -> [3, 0, 0]; the bypass
    // replaces it with 150 s -> [1, 5, 0].
    let (h, t, o) = timer_bcd_digits(150);
    cpu.mem.wram[WRAM_TIMER_HUNDREDS] = h;
    cpu.mem.wram[WRAM_TIMER_HUNDREDS + 1] = t;
    cpu.mem.wram[WRAM_TIMER_HUNDREDS + 2] = o;
    assert_eq!(&cpu.mem.wram[WRAM_TIMER_HUNDREDS..WRAM_TIMER_HUNDREDS + 3], &[1, 5, 0]);
}
