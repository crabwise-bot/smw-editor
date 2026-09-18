//! Headless credits-scene Layer 3 text renderer.
//!
//! Renders one ending enemy-name stripe's Layer-3 text (the 64×14 full
//! viewable area) with the real GFX2F 2bpp credits font, exactly like the
//! editor's WYSIWYG credits preview (`render_credits_grid_image`):
//!
//! 1. Parse `enemy_name_stripes[scene]` with `parse_title_stripe`.
//! 2. Split Layer-3 text commands (`$5000`-destined) from preserved non-L3
//!    commands with `split_credits_commands`.
//! 3. Build the grid with `TitleTileGrid::from_commands`.
//! 4. Rasterize with GFX2F tiles (white on black), skipping
//!    `TITLE_TILEMAP_BLANK` (`$38FC` / `!EmptyTile`).
//!
//! ```sh
//! cargo run --bin render_credits -- --scene=0 --out=/tmp/credits.png --rom=smw.smc
//! ```

use std::{env, path::Path};

use image::{ImageBuffer, Rgb};
use smwe_rom::title_stripe::{
    parse_title_stripe,
    split_credits_commands,
    TitleTileGrid,
    CREDITS_L3_FIRST_ROW,
    CREDITS_L3_LAST_ROW,
    TITLE_TILEMAP_BLANK,
    TITLE_TILEMAP_WIDTH,
};

/// Pixel scale (the docs screenshot is 2x: 1024×224 for the 64×14 grid).
const SCALE: u32 = 2;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/credits.png");
    let scene: usize = args.iter().find_map(|a| a.strip_prefix("--scene=")).and_then(|s| s.parse().ok()).unwrap_or(0);
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));

    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let stripe = rom
        .title_credits
        .enemy_name_stripes
        .get(scene)
        .ok_or_else(|| anyhow::anyhow!("no enemy name stripe for scene {scene}"))?;
    println!("credits stripe {scene}: {} bytes", stripe.len());

    let cmds = parse_title_stripe(stripe)?;
    let (_non_l3, l3) = split_credits_commands(&cmds);
    let grid = TitleTileGrid::from_commands(&l3);

    let gfx2f = rom.gfx.files.get(0x2F).ok_or_else(|| anyhow::anyhow!("no GFX2F in ROM"))?;
    println!("GFX2F: {} tiles", gfx2f.tiles.len());

    let (gw, gh) = (TITLE_TILEMAP_WIDTH as u32, (CREDITS_L3_LAST_ROW + 1) as u32);
    let (w, h) = (gw * 8 * SCALE, gh * 8 * SCALE);
    let mut img = ImageBuffer::<Rgb<u8>, _>::new(w, h);
    for px in img.pixels_mut() {
        *px = Rgb([0, 0, 0]);
    }
    for y in CREDITS_L3_FIRST_ROW..=CREDITS_L3_LAST_ROW {
        for x in 0..TITLE_TILEMAP_WIDTH {
            let word = grid.cells[y][x];
            if word == TITLE_TILEMAP_BLANK {
                continue;
            }
            let tile_idx = (word & 0x3FF) as usize;
            let Some(tile) = gfx2f.tiles.get(tile_idx) else { continue };
            let flip_x = word & 0x4000 != 0;
            let flip_y = word & 0x8000 != 0;
            for py in 0..8u32 {
                for px in 0..8u32 {
                    // Standard SNES flip semantics (matches the title
                    // renderer): flip bits mirror the tile; no flip by
                    // default. (An earlier version inverted this and
                    // produced mirrored text.)
                    let sx = if flip_x { 7 - px } else { px } as usize;
                    let sy = if flip_y { 7 - py } else { py } as usize;
                    if tile.color_indices[sy * 8 + sx] == 0 {
                        continue;
                    }
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            img.put_pixel(
                                x as u32 * 8 * SCALE + px * SCALE + dx,
                                y as u32 * 8 * SCALE + py * SCALE + dy,
                                Rgb([255, 255, 255]),
                            );
                        }
                    }
                }
            }
        }
    }

    img.save(output)?;
    println!("wrote {output} ({w}x{h}) scene {scene}");
    Ok(())
}
