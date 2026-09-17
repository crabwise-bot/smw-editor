//! Headless boss-sequence-text renderer.
//!
//! Rasterizes boss cutscene messages to a PNG using the real message-font
//! graphics (GFX2A, "Message Box Letters") and the parsed stripe tile data
//! from `smwe_rom::boss_text`. This is what the Boss Sequence Text editor's
//! preview shows.
//!
//! ```sh
//! cargo run --bin render_boss_text -- --out=docs/screenshots/boss-text.png --rom=smw.smc
//! ```

use std::{env, path::Path};

use image::{Rgb, RgbImage};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/boss-text.png");
    let rom_path =
        args.iter().find_map(|a| a.strip_prefix("--rom=")).map(Path::new).unwrap_or_else(|| Path::new("smw.smc"));

    let raw = std::fs::read(rom_path)?;
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let rom = smwe_rom::snes_utils::rom::Rom::new(rom_bytes)?;
    let boss_text = smwe_rom::boss_text::BossText::parse(&rom)?;
    let font = smwe_rom::message_raster::decompress_message_font(&rom)?;

    // Layout: one section per boss, each message as a rasterized strip.
    // Scale 2x for visibility. Each strip is 8px tall; messages have ~24
    // tiles max.
    let scale = 2u32;
    let tile_px = 8 * scale;
    let mut max_tiles = 0;
    for boss_msgs in &boss_text.messages {
        for msg in boss_msgs {
            max_tiles = max_tiles.max(msg.len());
        }
    }
    let strip_w = (max_tiles as u32) * tile_px;
    let strip_h = tile_px;
    let gap = 8u32;

    let mut total_h = 10u32;
    for boss_msgs in &boss_text.messages {
        total_h += 6 + (boss_msgs.len() as u32) * (strip_h + 2) + gap;
    }
    let img_w = strip_w + 20;
    let mut img = RgbImage::new(img_w, total_h);
    // Dark background.
    for p in img.pixels_mut() {
        *p = Rgb([16, 16, 24]);
    }

    let palette = [Rgb([0, 0, 0]), Rgb([255, 255, 255]), Rgb([170, 170, 170]), Rgb([220, 220, 220])];
    let mut y = 10u32;

    for boss_msgs in boss_text.messages.iter() {
        // Thin separator line between bosses.
        for x in 10..img_w - 10 {
            img.put_pixel(x, y, Rgb([80, 80, 100]));
        }
        y += 6;

        for msg in boss_msgs {
            let tiles = msg.char_bytes();
            for (i, &tile_idx) in tiles.iter().enumerate() {
                let tile = &font[(tile_idx & 0x7F) as usize % font.len()];
                for ty in 0..8 {
                    for tx in 0..8 {
                        let c = tile[ty * 8 + tx] as usize;
                        let color = palette[c.min(3)];
                        let px = 10 + (i as u32) * tile_px + (tx as u32) * scale;
                        let py = y + (ty as u32) * scale;
                        for sy in 0..scale {
                            for sx in 0..scale {
                                if px + sx < img_w && py + sy < total_h {
                                    img.put_pixel(px + sx, py + sy, color);
                                }
                            }
                        }
                    }
                }
            }
            y += strip_h + 2;
        }
        y += gap;
    }

    img.save(output)?;
    println!("Saved {output} ({}x{})", img_w, total_h);
    Ok(())
}
