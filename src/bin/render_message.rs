//! Headless message-box renderer.
//!
//! Runs one message through the REAL game routine (`CODE_05B1BC`) and dumps
//! the dynamic stripe image it appends to WRAM:
//!
//! ```sh
//! cargo run --bin render_message -- --message=0 --out=/tmp/msg0_stripe.bin --rom=smw.smc
//! ```
//!
//! What the routine does (verified in SMWDisX `bank_05.asm`): it does NOT
//! upload font graphics to VRAM. It appends 8 rows × 18 tile words to the WRAM
//! stripe buffer (`DynamicStripeImage` at $7F837D). Each tile word is `$39TT`
//! (tiles $100-$17F, palette 6, priority 1); the font graphics must already be
//! in VRAM from the game's normal GFX upload.
//!
//! Stripe command format (from `LoadStripeImage`, bank_00.asm):
//! ```text
//! [VRAM-dest word][flags/length word][payload bytes...]
//! ```
//! flags/length word: bit 15 = vertical, bit 14 = RLE, low 14 bits = payload
//! length in bytes minus 1. A first byte with bit 7 set ($FF) terminates.
//!
//! UNVERIFIED WITHOUT A ROM: `smwe_emu::emu::render_message` has never
//! executed (no SMW ROM on this machine). This binary writes the RAW stripe
//! bytes plus a decoded command listing — the input for the real-ROM
//! verification step, which rasterizes tiles $100-$17F (palette 6) at each
//! command's VRAM address into a PNG.

use std::{env, path::Path, sync::Arc};

use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

fn main() {
    let args: Vec<String> = env::args().collect();
    let message = args
        .iter()
        .find_map(|a| a.strip_prefix("--message="))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        message < smwe_rom::message_boxes::MESSAGE_COUNT,
        "message index {message} out of range (0-{})",
        smwe_rom::message_boxes::MESSAGE_COUNT - 1
    );
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/message_stripe.bin");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));

    let slot = smwe_rom::message_boxes::pointer_slot_for_message(message);
    println!(
        "rendering message {message} ({}) via pointer-table slot {slot}",
        smwe_rom::message_boxes::MESSAGE_NAMES[message]
    );

    let raw = std::fs::read(rom_path).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));

    let stripe = smwe_emu::emu::render_message(&mut cpu, slot);
    std::fs::write(output, &stripe.stripe).expect("write stripe snapshot");
    println!("wrote {output} ({} stripe bytes, {} cycles)", stripe.stripe.len(), stripe.cycles);

    // Decode the stripe commands so the output is human-checkable.
    let mut off = 0usize;
    let mut cmd = 0;
    while off < stripe.stripe.len() {
        let b0 = stripe.stripe[off];
        if b0 & 0x80 != 0 {
            println!("offset {off:#06X}: terminator byte {b0:#04X}");
            break;
        }
        assert!(off + 4 <= stripe.stripe.len(), "truncated stripe command at {off:#06X}");
        let dest = u16::from_le_bytes([stripe.stripe[off], stripe.stripe[off + 1]]);
        let flags_len = u16::from_le_bytes([stripe.stripe[off + 2], stripe.stripe[off + 3]]);
        let len = (flags_len & 0x3FFF) as usize + 1;
        let vertical = flags_len & 0x8000 != 0;
        let rle = flags_len & 0x4000 != 0;
        assert!(off + 4 + len <= stripe.stripe.len(), "truncated stripe payload at {off:#06X}");
        let payload = &stripe.stripe[off + 4..off + 4 + len];
        let tiles: Vec<String> =
            payload.chunks_exact(2).take(6).map(|w| format!("{:02X}{:02X}", w[1], w[0])).collect();
        println!(
            "cmd {cmd} at {off:#06X}: VRAM dest={dest:#06X}, {len} payload bytes, \
             vertical={vertical}, rle={rle}, first tiles=[{}…]",
            tiles.join(" ")
        );
        off += 4 + len;
        cmd += 1;
    }
    println!("{cmd} stripe command(s) decoded.");
    println!("NOTE: PNG rasterization is pending real-ROM verification — see");
    println!("      smwe_emu::emu::render_message docs for the remaining steps.");
}
