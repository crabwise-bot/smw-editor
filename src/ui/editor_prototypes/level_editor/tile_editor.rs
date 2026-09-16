//! 8x8 tile editor — Lunar Magic-style 8x8 tile pixel editing over GFX files.
//!
//! LM has had an 8x8 tile editor/selector since v1.90: a palette-colored grid
//! of every 8x8 tile in a GFX file, pixel-level painting with a palette color
//! picker, and a double-click handoff from the Map16 editor that jumps
//! straight to the tile under the cursor. This window implements that:
//!
//! - Tile grid: every 8x8 tile of the selected GFX file slot (native bit
//!   depth), colored with a selectable CGRAM palette row from the loaded
//!   level. Click a tile to select it.
//! - Pixel editor: left-drag paints with the selected palette color,
//!   right-click eyedrops the color under the cursor. "Apply" stages the tile
//!   into `tile_editor_staged` and re-encodes the whole file's raw bytes into
//!   `gfx_edits`, so the existing LC_LZ2 compress + repoint path in
//!   `save_to_rom` writes it back to the ROM.
//! - Handoff: `vram_tile_to_gfx_source` maps a Map16 tile word's VRAM tile
//!   number to its source GFX file using the level's ObjectTileset ($7E1931)
//!   and the game's OBJECTGFXLIST table ($00A92B — 26 rows of
//!   FG1/FG2/FG3/BG1 file numbers, already parsed as
//!   `smwe_rom::objects::object_gfx_list::ObjectGfxList`). That is the same
//!   layout the upload loop at `CODE_00AA35` produces: FG1 → VRAM tiles
//!   0x00-0x7F, FG2 → 0x80-0xFF, FG3 → 0x100-0x17F, BG1 → 0x180-0x1FF.

use egui::{pos2, vec2, Color32, Context, Rect, Sense, Slider};
use smwe_rom::graphics::gfx_file::{self, GfxFile, Tile, TileFormat};

use super::UiLevelEditor;

const GRID_COLS: usize = 16;
const GRID_SCALE: usize = 2; // each 8x8 tile drawn at 16x16 px
const PIXEL_CELL: f32 = 15.0; // pixel-editor cell size in UI px

/// Highest color index a tile format can store (2/3/4/8 bpp).
pub fn max_color_index(format: TileFormat) -> u8 {
    match format {
        TileFormat::Tile2bpp => 3,
        TileFormat::Tile3bpp | TileFormat::Tile3bppMode7 => 7,
        TileFormat::Tile4bpp => 15,
        TileFormat::Tile8bpp => 255,
    }
}

/// Decode one 16-color CGRAM palette row into egui colors (SNES BGR555).
pub fn cgram_palette_row(cgram: &[u8], row: usize) -> [Color32; 16] {
    let mut out = [Color32::BLACK; 16];
    for i in 0..16usize {
        let off = row * 32 + i * 2;
        if off + 1 < cgram.len() {
            let c = cgram[off] as u16 | ((cgram[off + 1] as u16) << 8);
            out[i] = Color32::from_rgb(
                ((c & 0x1F) << 3) as u8,
                (((c >> 5) & 0x1F) << 3) as u8,
                (((c >> 10) & 0x1F) << 3) as u8,
            );
        }
    }
    out
}

/// Render one 8x8 tile's color indices into an 8x8 RGBA buffer (`out` must be
/// 8*8*4 bytes) using the given palette. With `checker`, color index 0 is
/// drawn as a dark checkerboard (transparency preview) instead of palette
/// color 0.
pub fn tile_rgba8(tile: &Tile, palette: &[Color32; 16], checker: bool, out: &mut [u8]) {
    debug_assert!(out.len() >= 8 * 8 * 4);
    for py in 0..8usize {
        for px in 0..8usize {
            let idx = tile.color_indices.get(py * 8 + px).copied().unwrap_or(0);
            let off = (py * 8 + px) * 4;
            if idx == 0 && checker {
                let shade = if (px + py) % 2 == 0 { 46u8 } else { 74u8 };
                out[off] = shade;
                out[off + 1] = shade;
                out[off + 2] = shade;
                out[off + 3] = 255;
            } else {
                let c = palette[(idx as usize).min(15)];
                out[off] = c.r();
                out[off + 1] = c.g();
                out[off + 2] = c.b();
                out[off + 3] = 255;
            }
        }
    }
}

/// Which of the four FG/BG GFX slots a VRAM tile number belongs to
/// (FG1/FG2/FG3/BG1 = slots 0/1/2/3), or `None` when the tile is outside the
/// level's uploaded GFX region. Pure function, unit-tested below.
fn gfx_slot_for_tile(tile_num: u16) -> Option<usize> {
    if tile_num >= 0x200 {
        // Tiles 0x200-0x3FF are outside the FG/BG upload region: the game's
        // level upload loop (CODE_00AA35) only covers tiles 0x000-0x1FF.
        return None;
    }
    Some((tile_num / 0x80) as usize)
}

impl UiLevelEditor {
    pub(super) fn tile_editor_window(&mut self, ctx: &Context) {
        if !self.show_tile_editor {
            return;
        }
        let mut open = self.show_tile_editor;
        egui::Window::new("8x8 Tile Editor").open(&mut open).resizable(true).default_size([880.0, 560.0]).show(
            ctx,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label("GFX file:");
                    let mut file_num = self.tile_editor_file_num as i32;
                    let max = gfx_file::gfx_file_count() as i32 - 1;
                    if ui.add(Slider::new(&mut file_num, 0..=max).hexadecimal(2, false, false)).changed() {
                        self.tile_editor_file_num = file_num as usize;
                        self.tile_editor_selected = 0;
                        self.sync_tile_editor_pixels();
                    }
                    ui.label("Palette:");
                    let mut pal = self.tile_editor_palette as i32;
                    if ui.add(Slider::new(&mut pal, 0..=7)).changed() {
                        self.tile_editor_palette = pal as usize;
                    }
                });

                let file_num = self.tile_editor_file_num;
                let format = gfx_file::tile_format_of(file_num);
                let n_tiles = self.tile_editor_tiles(file_num).len();
                ui.horizontal(|ui| {
                    ui.label(format!("Format: {format}  •  {n_tiles} tiles"));
                    if self.tile_editor_staged.contains_key(&file_num) || self.gfx_edits.contains_key(&file_num) {
                        ui.colored_label(egui::Color32::from_rgb(220, 160, 60), "Unsaved edits staged for this file.");
                    }
                });
                if let Some(note) = self.tile_editor_handoff_note.clone() {
                    ui.small(&note);
                }
                ui.separator();

                ui.horizontal(|ui| {
                    egui::ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
                        self.tile_editor_grid(ui, file_num, n_tiles);
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        self.tile_editor_pixel_pane(ui, file_num, format);
                    });
                });
            },
        );
        self.show_tile_editor = open;
    }

    /// Tiles currently displayed for a file: the staged working copy when the
    /// user has applied pixel edits, otherwise the ROM-decoded tiles.
    fn tile_editor_tiles(&self, file_num: usize) -> &[Tile] {
        self.tile_editor_staged
            .get(&file_num)
            .map(Vec::as_slice)
            .or_else(|| self.rom.gfx.files.get(file_num).map(|f| f.tiles.as_slice()))
            .unwrap_or(&[])
    }

    /// (Re)load the pixel editor's working buffer from the selected tile.
    fn sync_tile_editor_pixels(&mut self) {
        let sel = self.tile_editor_selected;
        let file_num = self.tile_editor_file_num;
        let pixels: Option<[u8; 64]> =
            self.tile_editor_tiles(file_num).get(sel).and_then(|tile| tile.color_indices.as_ref().try_into().ok());
        match pixels {
            Some(p) => self.tile_editor_pixels = p,
            None => self.tile_editor_pixels = [0u8; 64],
        }
        self.tile_editor_dirty = false;
    }

    /// Open the editor at a specific file/tile (Map16 double-click handoff).
    pub(super) fn open_tile_editor_at(&mut self, file_num: usize, tile_idx: usize, palette: usize) {
        self.tile_editor_file_num = file_num;
        self.tile_editor_palette = palette.min(7);
        let n = self.tile_editor_tiles(file_num).len();
        self.tile_editor_selected = tile_idx.min(n.saturating_sub(1));
        self.sync_tile_editor_pixels();
        self.show_tile_editor = true;
    }

    /// Map a VRAM 8x8 tile number (Map16 tile word bits 0-9) to the GFX file
    /// and tile index it was uploaded from, or `None` when the tile has no
    /// GFX-file source in the current level (tiles 0x200-0x3FF, special
    /// tilesets, or a tile index past the end of the file).
    pub(super) fn vram_tile_to_gfx_source(&self, tile_num: u16) -> Option<(usize, usize)> {
        use smwe_rom::objects::map16::Tile8x8;
        let slot = gfx_slot_for_tile(tile_num)?;
        // ObjectTileset ($7E1931): row index into OBJECTGFXLIST, set by the
        // game's level init (runs inside decompress_sublevel).
        let tileset = *self.cpu.mem.wram.get(0x1931)? as usize;
        if tileset >= 26 {
            // Tilesets $FE/$FF take the game's special upload path
            // (CODE_00AB42 / SetallFGBG80), not the four-slot layout.
            return None;
        }
        debug_assert_eq!(slot, (tile_num / 0x80) as usize);
        let tile = Tile8x8(tile_num);
        let file_num = self.rom.gfx.object_gfx_list.gfx_file_for_object_tile(tile, tileset);
        let tile_in_file = (tile_num % 0x80) as usize;
        let n = self.rom.gfx.files.get(file_num)?.tiles.len();
        (tile_in_file < n).then_some((file_num, tile_in_file))
    }

    /// Double-click handoff from the Map16 Block Editor: jump to the 8x8 tile
    /// a sub-tile's graphics come from.
    pub(super) fn open_tile_editor_from_map16(&mut self, block_id: u16, sub_i: usize, tile_word: u16) {
        const SUB_NAMES: [&str; 4] = ["upper-left", "lower-left", "upper-right", "lower-right"];
        let tile_num = tile_word & 0x3FF;
        let palette = ((tile_word >> 10) & 0x7) as usize;
        match self.vram_tile_to_gfx_source(tile_num) {
            Some((file_num, tile_idx)) => {
                self.tile_editor_handoff_note = Some(format!(
                    "From Map16 block {block_id:#06X} {} (tile {tile_num:#05X}) → GFX file {file_num:02X} tile {tile_idx:#04X}",
                    SUB_NAMES[sub_i.min(3)],
                ));
                self.open_tile_editor_at(file_num, tile_idx, palette);
            }
            None => {
                self.tile_editor_handoff_note = Some(format!(
                    "Map16 block {block_id:#06X}: tile {tile_num:#05X} is outside the level's uploaded GFX region (tiles 0x000–0x1FF); nothing to edit."
                ));
                self.show_tile_editor = true;
            }
        }
    }

    /// Stage the pixel editor's working buffer: update the staged tile copy
    /// and re-encode the whole file into `gfx_edits` for `save_to_rom`.
    fn apply_tile_editor_pixels(&mut self) {
        let file_num = self.tile_editor_file_num;
        let sel = self.tile_editor_selected;
        let format = gfx_file::tile_format_of(file_num);
        let max_c = max_color_index(format);
        let mut tiles: Vec<Tile> = self.tile_editor_tiles(file_num).to_vec();
        let Some(tile) = tiles.get_mut(sel) else { return };
        let mut clamped = self.tile_editor_pixels;
        for v in clamped.iter_mut() {
            *v = (*v).min(max_c);
        }
        tile.color_indices = Box::new(clamped);
        let raw = GfxFile { tile_format: format, tiles: tiles.clone() }.to_raw_bytes();
        self.tile_editor_staged.insert(file_num, tiles);
        self.gfx_edits.insert(file_num, raw);
        self.tile_editor_revision += 1;
        self.sync_tile_editor_pixels();
        self.mark_edited();
    }

    /// Discard staged pixel edits (and any pending GFX import) for the
    /// current file, restoring the ROM-decoded tiles.
    fn revert_tile_editor_file(&mut self) {
        let file_num = self.tile_editor_file_num;
        self.tile_editor_staged.remove(&file_num);
        self.gfx_edits.remove(&file_num);
        self.tile_editor_revision += 1;
        self.sync_tile_editor_pixels();
    }

    /// Palette-colored grid of every 8x8 tile in the file; click to select.
    fn tile_editor_grid(&mut self, ui: &mut egui::Ui, file_num: usize, n_tiles: usize) {
        let rows = n_tiles.div_ceil(GRID_COLS).max(1);
        let key = (file_num, self.tile_editor_palette, self.tile_editor_revision);
        if self.tile_editor_grid_key != key || self.tile_editor_grid_tex.is_none() {
            let img = {
                let palette = cgram_palette_row(&self.cpu.mem.cgram, self.tile_editor_palette);
                let tiles = self.tile_editor_tiles(file_num);
                let mut img =
                    egui::ColorImage::new([GRID_COLS * 8 * GRID_SCALE, rows * 8 * GRID_SCALE], Color32::TRANSPARENT);
                let mut px = [0u8; 8 * 8 * 4];
                for (i, tile) in tiles.iter().enumerate() {
                    tile_rgba8(tile, &palette, false, &mut px);
                    let (tx, ty) = ((i % GRID_COLS) * 8 * GRID_SCALE, (i / GRID_COLS) * 8 * GRID_SCALE);
                    for sy in 0..8usize {
                        for sx in 0..8usize {
                            let src = (sy * 8 + sx) * 4;
                            let c = Color32::from_rgba_unmultiplied(px[src], px[src + 1], px[src + 2], px[src + 3]);
                            for dy in 0..GRID_SCALE {
                                for dx in 0..GRID_SCALE {
                                    img[(tx + sx * GRID_SCALE + dx, ty + sy * GRID_SCALE + dy)] = c;
                                }
                            }
                        }
                    }
                }
                img
            };
            let tex = ui.ctx().load_texture(format!("tile_editor_grid_{file_num}"), img, egui::TextureOptions::NEAREST);
            self.tile_editor_grid_tex = Some(tex);
            self.tile_editor_grid_key = key;
        }

        let tex = self.tile_editor_grid_tex.clone().expect("grid texture built above");
        let cell = 8.0 * GRID_SCALE as f32;
        let (rect, response) =
            ui.allocate_exact_size(vec2(GRID_COLS as f32 * cell, rows as f32 * cell), Sense::click());
        ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

        // Selection highlight.
        let sel = self.tile_editor_selected;
        if sel < n_tiles {
            let sel_rect = Rect::from_min_size(
                rect.min + vec2((sel % GRID_COLS) as f32 * cell, (sel / GRID_COLS) as f32 * cell),
                vec2(cell, cell),
            );
            ui.painter().rect_stroke(
                sel_rect,
                0.0,
                egui::Stroke::new(2.0_f32, Color32::YELLOW),
                egui::StrokeKind::Outside,
            );
        }

        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let lx = ((pos.x - rect.min.x) / cell) as isize;
                let ly = ((pos.y - rect.min.y) / cell) as isize;
                if lx >= 0 && ly >= 0 {
                    let idx = ly as usize * GRID_COLS + lx as usize;
                    if idx < n_tiles {
                        self.tile_editor_selected = idx;
                        self.sync_tile_editor_pixels();
                    }
                }
            }
        }
        if response.hovered() {
            if let Some(pos) = response.hover_pos() {
                let lx = ((pos.x - rect.min.x) / cell) as isize;
                let ly = ((pos.y - rect.min.y) / cell) as isize;
                if lx >= 0 && ly >= 0 {
                    let idx = ly as usize * GRID_COLS + lx as usize;
                    if idx < n_tiles {
                        response.on_hover_text(format!("Tile {idx:#04X}"));
                    }
                }
            }
        }
    }

    /// Zoomed pixel editor for the selected tile: paint, eyedrop, apply.
    fn tile_editor_pixel_pane(&mut self, ui: &mut egui::Ui, file_num: usize, format: TileFormat) {
        let sel = self.tile_editor_selected;
        ui.label(format!("Tile {sel:#04X} of GFX file {file_num:02X}"));
        let palette = cgram_palette_row(&self.cpu.mem.cgram, self.tile_editor_palette);
        let max_c = max_color_index(format);

        let (rect, response) =
            ui.allocate_exact_size(vec2(8.0 * PIXEL_CELL, 8.0 * PIXEL_CELL), Sense::click_and_drag());
        let painter = ui.painter();
        for py in 0..8usize {
            for px in 0..8usize {
                let idx = self.tile_editor_pixels[py * 8 + px];
                let cell = Rect::from_min_size(
                    rect.min + vec2(px as f32 * PIXEL_CELL, py as f32 * PIXEL_CELL),
                    vec2(PIXEL_CELL, PIXEL_CELL),
                );
                if idx == 0 {
                    let shade = if (px + py) % 2 == 0 { 46u8 } else { 74u8 };
                    painter.rect_filled(cell, 0.0, Color32::from_gray(shade));
                } else {
                    painter.rect_filled(cell, 0.0, palette[(idx as usize).min(15)]);
                }
                painter.rect_stroke(
                    cell,
                    0.0,
                    egui::Stroke::new(0.5_f32, Color32::from_black_alpha(90)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        let pixel_at = |pos: egui::Pos2| -> Option<usize> {
            let px = ((pos.x - rect.min.x) / PIXEL_CELL) as isize;
            let py = ((pos.y - rect.min.y) / PIXEL_CELL) as isize;
            if (0..8).contains(&px) && (0..8).contains(&py) {
                Some(py as usize * 8 + px as usize)
            } else {
                None
            }
        };

        // Paint while the primary button is held down over the grid.
        if ui.input(|i| i.pointer.primary_down()) {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(i) = pixel_at(pos) {
                    let c = self.tile_editor_paint_color.min(max_c);
                    if self.tile_editor_pixels[i] != c {
                        self.tile_editor_pixels[i] = c;
                        self.tile_editor_dirty = true;
                    }
                }
            }
        }
        // Right-click eyedrops the color under the cursor.
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(i) = pixel_at(pos) {
                    self.tile_editor_paint_color = self.tile_editor_pixels[i].min(max_c);
                }
            }
        }

        ui.add_space(6.0);
        ui.label(format!("Paint color (0–{max_c}, {format}):"));
        ui.horizontal_wrapped(|ui| {
            for i in 0..=max_c {
                let c = palette[i as usize];
                let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                if i == 0 {
                    // checkerboard swatch for the transparent index
                    for cy in 0..2 {
                        for cx in 0..2 {
                            let shade = if (cx + cy) % 2 == 0 { 46u8 } else { 74u8 };
                            ui.painter().rect_filled(
                                Rect::from_min_size(r.min + vec2(cx as f32 * 12.0, cy as f32 * 12.0), vec2(12.0, 12.0)),
                                0.0,
                                Color32::from_gray(shade),
                            );
                        }
                    }
                } else {
                    ui.painter().rect_filled(r, 2.0, c);
                }
                if i == self.tile_editor_paint_color {
                    ui.painter().rect_stroke(
                        r,
                        2.0,
                        egui::Stroke::new(2.0_f32, Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                }
                if resp.clicked() {
                    self.tile_editor_paint_color = i;
                }
                resp.on_hover_text(format!("Color {i}"));
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.tile_editor_dirty, egui::Button::new("Apply pixel edits")).clicked() {
                self.apply_tile_editor_pixels();
            }
            if ui.button("Revert file").clicked() {
                self.revert_tile_editor_file();
            }
        });
        ui.small("Left-drag paints • right-click picks up a color • Apply stages the tile for save.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile_with_indices(color_indices: [u8; 64]) -> Tile {
        Tile { color_indices: Box::new(color_indices) }
    }

    #[test]
    fn max_color_index_matches_bit_depth() {
        use TileFormat::*;
        assert_eq!(max_color_index(Tile2bpp), 3);
        assert_eq!(max_color_index(Tile3bpp), 7);
        assert_eq!(max_color_index(Tile3bppMode7), 7);
        assert_eq!(max_color_index(Tile4bpp), 15);
        assert_eq!(max_color_index(Tile8bpp), 255);
    }

    #[test]
    fn cgram_palette_row_decodes_bgr555() {
        // CGRAM entry 0x7FFF = white, 0x001F = full red.
        let mut cgram = [0u8; 512];
        cgram[0] = 0xFF;
        cgram[1] = 0x7F;
        cgram[2] = 0x1F;
        cgram[3] = 0x00;
        let pal = cgram_palette_row(&cgram, 0);
        assert_eq!(pal[0], Color32::from_rgb(0xF8, 0xF8, 0xF8));
        assert_eq!(pal[1], Color32::from_rgb(0xF8, 0x00, 0x00));
        // Row 1 reads a different CGRAM region.
        let pal1 = cgram_palette_row(&cgram, 1);
        assert_eq!(pal1[0], Color32::BLACK);
    }

    #[test]
    fn tile_rgba8_maps_indices_through_palette() {
        let mut indices = [0u8; 64];
        indices[0] = 1;
        indices[9] = 5;
        let tile = tile_with_indices(indices);
        let mut palette = [Color32::BLACK; 16];
        palette[1] = Color32::from_rgb(10, 20, 30);
        palette[5] = Color32::from_rgb(200, 100, 50);
        let mut out = [0u8; 8 * 8 * 4];
        tile_rgba8(&tile, &palette, false, &mut out);
        assert_eq!(&out[0..4], &[10, 20, 30, 255]);
        assert_eq!(&out[9 * 4..9 * 4 + 4], &[200, 100, 50, 255]);
        // Index 0 renders palette color 0 without the checkerboard flag.
        assert_eq!(&out[1 * 4..1 * 4 + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn tile_rgba8_checkerboard_replaces_transparent_index() {
        let tile = tile_with_indices([0u8; 64]);
        let palette = [Color32::from_rgb(255, 0, 0); 16];
        let mut out = [0u8; 8 * 8 * 4];
        tile_rgba8(&tile, &palette, true, &mut out);
        // (0,0) and (1,1) are the dark squares, (1,0) the light one.
        assert_eq!(&out[0..3], &[46, 46, 46]);
        assert_eq!(&out[1 * 4..1 * 4 + 3], &[74, 74, 74]);
        assert_eq!(&out[9 * 4..9 * 4 + 3], &[46, 46, 46]);
    }

    #[test]
    fn gfx_slot_for_tile_matches_upload_layout() {
        // FG1/FG2/FG3/BG1 slots per the CODE_00AA35 upload loop.
        assert_eq!(gfx_slot_for_tile(0x00), Some(0));
        assert_eq!(gfx_slot_for_tile(0x7F), Some(0));
        assert_eq!(gfx_slot_for_tile(0x80), Some(1));
        assert_eq!(gfx_slot_for_tile(0xFF), Some(1));
        assert_eq!(gfx_slot_for_tile(0x100), Some(2));
        assert_eq!(gfx_slot_for_tile(0x17F), Some(2));
        assert_eq!(gfx_slot_for_tile(0x180), Some(3));
        assert_eq!(gfx_slot_for_tile(0x1FF), Some(3));
        // Outside the uploaded region: no GFX-file source.
        assert_eq!(gfx_slot_for_tile(0x200), None);
        assert_eq!(gfx_slot_for_tile(0x3FF), None);
    }

    #[test]
    fn tile_index_within_file_is_tile_mod_0x80() {
        // The upload loop writes each file's 0x80 tiles contiguously, so the
        // tile's index inside its file is the VRAM tile number mod 0x80.
        for tile_num in [0x00u16, 0x45, 0x80, 0x123, 0x180, 0x1FF] {
            let slot = gfx_slot_for_tile(tile_num).unwrap();
            assert_eq!(tile_num as usize % 0x80, tile_num as usize - slot * 0x80);
        }
    }
}

#[cfg(test)]
mod real_rom_tests {
    use smwe_rom::{objects::map16::Tile8x8, SmwRom};

    /// The OBJECTGFXLIST table the handoff mapping relies on must start at
    /// $00A92B with the documented "Normal 1" row ($14,$17,$19,$15) — the
    /// disassembly's symbol label ($00A930) is 5 bytes off, so pin the real
    /// address. Run with `ROM_PATH=/path/to/smw.smc cargo test --lib
    /// -- --ignored object_gfx_list_table_address`.
    #[test]
    #[ignore]
    fn object_gfx_list_table_address() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(rom_path).expect("parse ROM");
        let row0 = (0..4)
            .map(|slot| rom.gfx.object_gfx_list.gfx_file_for_object_tile(Tile8x8(slot as u16 * 0x80), 0))
            .collect::<Vec<_>>();
        assert_eq!(row0, vec![0x14, 0x17, 0x19, 0x15], "tileset 0 must be the 'Normal 1' row");
        // Every referenced file must exist and hold at least 0x80 tiles (one
        // upload slot's worth).
        for file_num in row0 {
            assert!(rom.gfx.files[file_num].tiles.len() >= 0x80, "file {file_num:02X} too small for an upload slot");
        }
    }
}
