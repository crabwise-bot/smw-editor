//! Headless message-box renderer.
//!
//! Runs one message through the REAL game routine (`CODE_05B1BC`), parses the
//! dynamic stripe image it appends to WRAM, and rasterizes the 8×18 tile grid
//! to a PNG using the real message-font graphics (GFX2A, "Message Box
//! Letters").
//!
//! ```sh
//! cargo run --bin render_message -- --message=0 --out=/tmp/msg0.png --rom=smw.smc
//! ```
//!
//! What the routine does (verified in SMWDisX `bank_05.asm`): it does NOT
//! upload font graphics to VRAM. It appends 8 rows × 18 tile words to the WRAM
//! stripe buffer (`DynamicStripeImage` at $7F837D). Each tile word is `$39TT`
//! (tiles $100-$17F, palette 6, priority 1). The font graphics (GFX2A, 2bpp,
//! 128 tiles) are decompressed from the ROM and used to rasterize the PNG.

use std::{env, path::Path, sync::Arc};

use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let message =
        args.iter().find_map(|a| a.strip_prefix("--message=")).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    anyhow::ensure!(
        message < smwe_rom::message_boxes::MESSAGE_COUNT,
        "message index {message} out of range (0-{})",
        smwe_rom::message_boxes::MESSAGE_COUNT - 1
    );
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/message.png");
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

    // Run the real CODE_05B1BC via the emulator.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    let stripe = smwe_emu::emu::render_message(&mut cpu, slot);
    println!("CODE_05B1BC ran: {} stripe bytes, {} cycles", stripe.stripe.len(), stripe.cycles);

    // Parse the stripe with the shared big-endian-header parser.
    let cmds = smwe_emu::emu::parse_stripe_commands(&stripe.stripe).map_err(|e| anyhow::anyhow!(e))?;
    anyhow::ensure!(cmds.len() == 8, "expected 8 stripe commands, found {}", cmds.len());
    for (i, cmd) in cmds.iter().enumerate() {
        anyhow::ensure!(cmd.tiles.len() == 18, "command {i}: expected 18 tiles, found {}", cmd.tiles.len());
        for &t in &cmd.tiles {
            anyhow::ensure!(t & 0xFF00 == 0x3900, "command {i}: unexpected tile word {t:#06X}");
        }
    }
    println!("stripe OK: 8 commands × 18 tiles, all attributes $39");

    // Build the 8×18 tile-index grid from the stripe (tile word $39TT → $TT).
    let mut cells = [[0u8; 18]; 8];
    for (r, cmd) in cmds.iter().enumerate() {
        for (c, &t) in cmd.tiles.iter().enumerate() {
            cells[r][c] = (t & 0xFF) as u8;
        }
    }

    // Cross-check against the row-aware decoder: they must agree exactly.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let msg_bytes = &rom.message_boxes.messages[message];
    let expected = smwe_rom::font_map::message_cells(msg_bytes);
    anyhow::ensure!(cells == expected, "stripe grid disagrees with message_cells decoder");

    // Rasterize with the real GFX2A font graphics.
    let font = smwe_rom::message_raster::decompress_message_font(&rom.rom)?;
    let img = smwe_rom::message_raster::rasterize_message(cells, &font);
    // Scale 4x for visibility.
    let big = image::imageops::resize(&img, img.width() * 4, img.height() * 4, image::imageops::FilterType::Nearest);
    big.save(output)?;
    println!("wrote {output} ({}×{} PNG, 4x scale)", big.width(), big.height());
    Ok(())
}
