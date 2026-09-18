use std::{env, path::Path};

use smw_editor::level_png_export::{level_png_bytes, load_level_cpu, LevelPngOptions};
use smwe_emu::Cpu;
fn main() {
    let args: Vec<String> = env::args().collect();
    let level = args
        .iter()
        .find_map(|a| a.strip_prefix("--level="))
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x105);
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/level.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));
    let inspect = args.iter().find_map(|a| a.strip_prefix("--inspect=")).and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some((x.parse::<u32>().ok()?, y.parse::<u32>().ok()?))
    });

    let raw = std::fs::read(rom_path).expect("cannot read ROM");

    // The pixel data comes from the shared library pipeline so the binary and
    // the File-menu export can never drift apart.
    let layer = args.iter().find_map(|a| a.strip_prefix("--layer="));
    let opts = LevelPngOptions {
        include_layer1:  layer.map_or(true, |l| l != "2"),
        include_layer2:  layer.map_or(true, |l| l != "1"),
        include_sprites: !args.iter().any(|a| a == "--no-sprites"),
    };
    let png = level_png_bytes(&raw, level, &opts).expect("render level to PNG");
    std::fs::write(output, &png).expect("save png");
    println!("wrote {output}");

    if let Some((x, y)) = inspect {
        let mut cpu = load_level_cpu(&raw, level).expect("load level CPU");
        inspect_block(&mut cpu, false, x, y);
        inspect_block(&mut cpu, true, x, y);
    }
}

fn inspect_block(cpu: &mut Cpu, bg: bool, block_x_wanted: u32, block_y_wanted: u32) {
    let map16_bank = cpu.mem.cart.resolve("Map16Common").expect("Cannot resolve Map16Common") & 0xFF0000;
    let mut scratch = cpu.clone();
    let map16_bg = smwe_emu::emu::lm_bg_map16_base(&mut scratch)
        .unwrap_or_else(|| cpu.mem.cart.resolve("Map16BGTiles").expect("Cannot resolve Map16BGTiles"));
    let vertical = cpu.mem.load_u8(0x5B) & if bg { 2 } else { 1 } != 0;
    let mode = cpu.mem.load_u8(0x1925);
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").unwrap() + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));
    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    };
    let scr_size = if vertical { 16 * 32 } else { 16 * 27 };
    let (blocks_lo_addr, blocks_hi_addr) = match (bg, has_layer2) {
        (true, true) => {
            let offset = scr_len * scr_size;
            (0x7EC800 + offset, 0x7FC800 + offset)
        }
        (true, false) => (0x7EB900, 0x7EBD00),
        (false, _) => (0x7EC800, 0x7FC800),
    };
    let len = if has_layer2 { 256 * 27 } else { 512 * 27 };
    for idx in 0..len {
        let (block_x, block_y) = if vertical {
            let (screen, sidx) = (idx / (16 * 16), idx % (16 * 16));
            let (row, column) = (sidx / 16, sidx % 16);
            let (sub_y, sub_x) = (screen / 2, screen % 2);
            (column * 16 + sub_x * 256, row * 16 + sub_y * 256)
        } else {
            let (screen, sidx) = (idx / (16 * 27), idx % (16 * 27));
            let (row, column) = (sidx / 16, sidx % 16);
            (column * 16 + screen * 256, row * 16)
        };
        if block_x != block_x_wanted || block_y != block_y_wanted {
            continue;
        }
        let idx_adj = if bg && !has_layer2 { idx % (16 * 27 * 2) } else { idx };
        let lo = cpu.mem.load_u8(blocks_lo_addr + idx_adj) as u16;
        let hi_raw = cpu.mem.load_u8(blocks_hi_addr + idx_adj) as u16;
        let block_id = lo | ((hi_raw & 0x3F) << 8);
        if block_id == 0 {
            println!(
                "{} ({:03},{:03}) idx={} idx_adj={} lo={:02X} hi={:02X} block=000 <empty>",
                if bg { "L2" } else { "L1" },
                block_x,
                block_y,
                idx,
                idx_adj,
                lo,
                hi_raw
            );
            break;
        }
        let block_ptr = if bg && !has_layer2 {
            block_id as u32 * 8 + map16_bg
        } else if block_id >= 0x200 {
            smwe_emu::emu::lm_ext_map16_data_addr(&mut scratch, block_id).unwrap_or(0)
        } else {
            cpu.mem.load_u16(0x0FBE + block_id as u32 * 2) as u32 + map16_bank
        };
        println!(
            "{} ({:03},{:03}) idx={} idx_adj={} lo={:02X} hi={:02X} block={:03X} ptr={:06X}",
            if bg { "L2" } else { "L1" },
            block_x,
            block_y,
            idx,
            idx_adj,
            lo,
            hi_raw,
            block_id,
            block_ptr
        );
        for sub in 0..4u32 {
            let t = cpu.mem.load_u16(block_ptr + sub * 2);
            println!("  sub{}={:04X}", sub, t);
        }
        break;
    }
}
