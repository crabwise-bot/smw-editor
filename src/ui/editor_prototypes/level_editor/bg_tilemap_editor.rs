//! Lunar Magic-style dedicated Background Tile Map Editor.
//!
//! WYSIWYG editor for the level's legacy Layer 2 background tilemap: a 32x27
//! grid of Map16 block IDs rendered with the level's real BG GFX and palette.
//! Matches Lunar Magic's Background Tile Map Editor: 1x/2x zoom, paint/select/
//! eyedropper tools, undo/redo, Shift+Right-click pattern fill, resize of the
//! selection as a repeating pattern, Select All, tile grid toggle, and the
//! four bank/tile operations (Change Background Map16 Bank, Remap Background
//! Tiles, Copy Background Image, Add Offset to Background Tiles).
//!
//! Each tilemap entry is a Map16 block number *within the background's Map16
//! bank (page)*. The bank itself comes from the game's own rule: a Layer 2
//! pointer below SNES `$0CE8FE` means page 0, at/above means page 1
//! (SMWDisX `bank_05.asm`, `CODE_058126`).

use std::collections::HashSet;

use egui::{Color32, Context, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};
use smwe_emu::Cpu;
use smwe_rom::level::background::{bg_cell_index, bg_tile_offset, BG_TILEMAP_HEIGHT, BG_TILEMAP_LEN, BG_TILEMAP_WIDTH};

use super::{
    editing::{drag_handle_rects, handle_at, DragHandle},
    tile_picker::render_sub_tile,
    UiLevelEditor,
};

// Canvas size in pixels at 1x zoom (32x27 cells of 16x16).
const CANVAS_W: u32 = 512;
const CANVAS_H: u32 = 432;
// Tile selector: 16x16 blocks of 16x16 px.
const SEL_COLS: u32 = 16;
const SEL_CELL: u32 = 16;

/// Editing tool in the background tile map editor.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum BgTileTool {
    #[default]
    Paint,
    Select,
    Eyedropper,
}

/// In-progress pointer gesture on the canvas. Paint strokes and selection
/// resizes are transient: the canvas shows a live preview while dragging and
/// a single undoable write is committed on release, so one drag == one undo
/// step (the same pattern as the Lunar Magic-style object drags).
#[derive(Clone)]
pub(super) enum BgDrag {
    Paint {
        cells: Vec<(u32, u32)>,
    },
    Select {
        start: (u32, u32),
        cur:   (u32, u32),
    },
    Resize {
        handle:  DragHandle,
        orig:    (u32, u32, u32, u32),
        cur:     (u32, u32, u32, u32),
        /// Tile values of the selection when the drag started.
        content: Vec<u8>,
        cw:      u32,
        ch:      u32,
    },
}

// -------------------------------------------------------------------------------------------------
// Rendering / commit helpers
// -------------------------------------------------------------------------------------------------

impl UiLevelEditor {
    /// Read all 512 BG Map16 blocks (pages 0+1) as 4 tile words each.
    /// Pure ROM data, so it is cached per level; pixels are rendered from it
    /// together with the live VRAM/CGRAM.
    pub(super) fn bg_map16_block_words(cpu: &mut Cpu) -> Vec<[u16; 4]> {
        crate::render_util::bg_map16_block_words(cpu)
    }

    /// Render `tiles` (32x27 Map16 block IDs) to 512x432 RGBA pixels using the
    /// real BG Map16 table, VRAM and CGRAM. Block 0 is the erase/empty tile
    /// and shows the backdrop color, like the main level view.
    fn bg_render_pixels(&self, tiles: &[u8]) -> Vec<u8> {
        crate::render_util::render_bg_tilemap(
            tiles,
            self.bg_page,
            &self.bg_block_words,
            &self.cpu.mem.vram,
            &self.cpu.mem.cgram,
        )
    }

    /// Tile values the canvas should show right now: the live drag preview
    /// while a paint/resize gesture is in progress, else the committed model.
    fn bg_preview_tiles(&self) -> Option<Vec<u8>> {
        let drag = self.bg_drag.as_ref()?;
        let mut tiles = self.layer2_background.as_ref()?.read(|l| l.tile_ids.clone());
        match drag {
            BgDrag::Paint { cells } => {
                for &(c, r) in cells {
                    if let Some(idx) = bg_cell_index(c, r) {
                        tiles[idx] = self.bg_selected_tile;
                    }
                }
            }
            BgDrag::Resize { orig: (ox, oy, ow, oh), cur: (nx, ny, nw, nh), content, cw, ch, .. } => {
                for r in 0..*oh {
                    for c in 0..*ow {
                        if let Some(idx) = bg_cell_index(ox + c, oy + r) {
                            tiles[idx] = 0;
                        }
                    }
                }
                for r in 0..*nh {
                    for c in 0..*nw {
                        if let Some(idx) = bg_cell_index(nx + c, ny + r) {
                            tiles[idx] = content[((r % ch) * cw + (c % cw)) as usize];
                        }
                    }
                }
            }
            BgDrag::Select { .. } => return None,
        }
        Some(tiles)
    }

    pub(super) fn bg_rebuild_canvas(&mut self, ctx: &Context) {
        let tiles = match self.bg_preview_tiles() {
            Some(preview) => preview,
            None => match &self.layer2_background {
                Some(bg) => bg.read(|l| l.tile_ids.clone()),
                None => return,
            },
        };
        let pixels = self.bg_render_pixels(&tiles);
        let image = egui::ColorImage::from_rgba_unmultiplied([CANVAS_W as usize, CANVAS_H as usize], &pixels);
        match self.bg_canvas_tex.as_mut() {
            Some(tex) => tex.set(image, TextureOptions::NEAREST),
            None => {
                self.bg_canvas_tex = Some(ctx.load_texture("bg_tilemap_canvas", image, TextureOptions::NEAREST));
            }
        }
    }

    fn bg_rebuild_selector(&mut self, ctx: &Context) {
        let (w, h) = (SEL_COLS * SEL_CELL, 16 * SEL_CELL);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for px in pixels.chunks_exact_mut(4) {
            px[0] = 0x1a;
            px[1] = 0x1a;
            px[2] = 0x1a;
            px[3] = 255;
        }
        let vram = &self.cpu.mem.vram;
        let cgram = &self.cpu.mem.cgram;
        let page = self.bg_page as usize;
        let sub = [(0u32, 0u32), (0, 8), (8, 0), (8, 8)];
        for tile in 0..256u32 {
            let words = self.bg_block_words.get(page * 256 + tile as usize).copied().unwrap_or([0; 4]);
            let (x0, y0) = ((tile % SEL_COLS) * SEL_CELL, (tile / SEL_COLS) * SEL_CELL);
            for (k, (dx, dy)) in sub.iter().enumerate() {
                render_sub_tile(vram, cgram, words[k], x0 + dx, y0 + dy, &mut pixels, w as usize);
            }
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
        match self.bg_selector_tex.as_mut() {
            Some(tex) => tex.set(image, TextureOptions::NEAREST),
            None => {
                self.bg_selector_tex = Some(ctx.load_texture("bg_tilemap_selector", image, TextureOptions::NEAREST));
            }
        }
    }

    /// Apply tile edits as a single undo step, then sync the emulator tilemap
    /// RAM (what the main canvas renders), refresh the WYSIWYG canvas and
    /// mark the ROM dirty.
    pub(super) fn bg_commit_tiles(&mut self, edits: &[(usize, u8)]) {
        if edits.is_empty() {
            return;
        }
        if let Some(bg) = self.layer2_background.as_mut() {
            bg.write(|layer| {
                for &(idx, val) in edits {
                    if idx < layer.tile_ids.len() {
                        layer.tile_ids[idx] = val;
                    }
                }
            });
        }
        self.bg_after_edit();
    }

    /// Sync WRAM tilemap RAM + refresh canvas + mark dirty (no model change).
    /// Used after undo/redo and after bank switches.
    fn bg_after_edit(&mut self) {
        if let Some(bg) = &self.layer2_background {
            let tiles = bg.read(|l| l.tile_ids.clone());
            let page = self.bg_page;
            for (i, t) in tiles.iter().enumerate().take(BG_TILEMAP_LEN) {
                self.cpu.mem.store_u8(0x7EB900 + i as u32, *t);
                self.cpu.mem.store_u8(0x7EBD00 + i as u32, page);
            }
        }
        self.rebuild_tiles();
        self.bg_canvas_dirty = true;
        self.mark_edited();
    }

    fn bg_undo(&mut self) {
        if let Some(bg) = self.layer2_background.as_mut() {
            bg.undo();
        }
        self.bg_after_edit();
        self.bg_status = Some("Undid background edit.".to_string());
    }

    fn bg_redo(&mut self) {
        if let Some(bg) = self.layer2_background.as_mut() {
            bg.redo();
        }
        self.bg_after_edit();
        self.bg_status = Some("Redid background edit.".to_string());
    }

    fn bg_pick_tile(&mut self, col: u32, row: u32) {
        let idx = match bg_cell_index(col, row) {
            Some(idx) => idx,
            None => return,
        };
        if let Some(bg) = &self.layer2_background {
            let t = bg.read(|l| l.tile_ids[idx]);
            self.bg_selected_tile = t;
            self.bg_status = Some(format!("Picked tile ${t:02X}."));
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tile operations
// -------------------------------------------------------------------------------------------------

impl UiLevelEditor {
    /// The fill pattern: the current selection's content, or the single
    /// selected tile when nothing is selected.
    fn bg_pattern(&self) -> (u32, u32, Vec<u8>) {
        if let Some((x, y, w, h)) = self.bg_selection {
            let tiles = self.layer2_background.as_ref().map(|bg| bg.read(|l| l.tile_ids.clone())).unwrap_or_default();
            let mut pat = Vec::with_capacity((w * h) as usize);
            for r in 0..h {
                for c in 0..w {
                    pat.push(tiles[bg_cell_index(x + c, y + r).unwrap_or(0).min(BG_TILEMAP_LEN - 1)]);
                }
            }
            (w, h, pat)
        } else {
            (1, 1, vec![self.bg_selected_tile])
        }
    }

    /// Lunar Magic's Shift+Right-click: flood-fill the contiguous region
    /// holding the clicked cell's value with the pattern tiled from the
    /// clicked cell.
    fn bg_flood_fill(&mut self, col: u32, row: u32) {
        let start_idx = match bg_cell_index(col, row) {
            Some(idx) => idx,
            None => return,
        };
        let tiles = match &self.layer2_background {
            Some(bg) => bg.read(|l| l.tile_ids.clone()),
            None => return,
        };
        let target = tiles[start_idx];
        let (pw, ph, pat) = self.bg_pattern();
        if pat.iter().all(|&v| v == target) {
            self.bg_status = Some("Pattern fill: the pattern already matches the area.".to_string());
            return;
        }
        let mut filled = vec![false; BG_TILEMAP_LEN];
        let mut stack = vec![(col, row)];
        let mut edits = Vec::new();
        while let Some((c, r)) = stack.pop() {
            let idx = match bg_cell_index(c, r) {
                Some(idx) => idx,
                None => continue,
            };
            if filled[idx] || tiles[idx] != target {
                continue;
            }
            filled[idx] = true;
            let dx = (c as i32 - col as i32).rem_euclid(pw as i32) as usize;
            let dy = (r as i32 - row as i32).rem_euclid(ph as i32) as usize;
            let v = pat[dy * pw as usize + dx];
            if v != tiles[idx] {
                edits.push((idx, v));
            }
            if c > 0 {
                stack.push((c - 1, r));
            }
            if c + 1 < BG_TILEMAP_WIDTH as u32 {
                stack.push((c + 1, r));
            }
            if r > 0 {
                stack.push((c, r - 1));
            }
            if r + 1 < BG_TILEMAP_HEIGHT as u32 {
                stack.push((c, r + 1));
            }
        }
        if edits.is_empty() {
            self.bg_status = Some("Pattern fill: nothing to change.".to_string());
        } else {
            let n = edits.len();
            self.bg_commit_tiles(&edits);
            self.bg_status = Some(format!("Pattern-filled {n} tiles."));
        }
    }

    /// Clear the selected rectangle (set to empty), one undo step.
    fn bg_clear_selection(&mut self) {
        let Some((x, y, w, h)) = self.bg_selection else { return };
        let tiles = match &self.layer2_background {
            Some(bg) => bg.read(|l| l.tile_ids.clone()),
            None => return,
        };
        let mut edits = Vec::new();
        for r in 0..h {
            for c in 0..w {
                if let Some(idx) = bg_cell_index(x + c, y + r) {
                    if tiles[idx] != 0 {
                        edits.push((idx, 0));
                    }
                }
            }
        }
        if edits.is_empty() {
            self.bg_status = Some("Selection is already empty.".to_string());
        } else {
            let n = edits.len();
            self.bg_commit_tiles(&edits);
            self.bg_status = Some(format!("Cleared {n} tiles."));
        }
    }

    /// Commit a finished resize drag: the new rectangle is filled by tiling
    /// the snapshot content as a repeating pattern; cells of the old
    /// rectangle left outside the new one are cleared.
    fn bg_commit_resize(
        &mut self, orig: (u32, u32, u32, u32), cur: (u32, u32, u32, u32), content: &[u8], cw: u32, ch: u32,
    ) {
        let (ox, oy, ow, oh) = orig;
        let (nx, ny, nw, nh) = cur;
        if (nx, ny, nw, nh) == (ox, oy, ow, oh) {
            self.bg_canvas_dirty = true;
            return;
        }
        let mut edits = Vec::new();
        for r in 0..oh {
            for c in 0..ow {
                let (cc, rr) = (ox + c, oy + r);
                if cc < nx || cc >= nx + nw || rr < ny || rr >= ny + nh {
                    if let Some(idx) = bg_cell_index(cc, rr) {
                        edits.push((idx, 0));
                    }
                }
            }
        }
        for r in 0..nh {
            for c in 0..nw {
                let v = content[((r % ch) * cw + (c % cw)) as usize];
                if let Some(idx) = bg_cell_index(nx + c, ny + r) {
                    edits.push((idx, v));
                }
            }
        }
        self.bg_commit_tiles(&edits);
        self.bg_selection = Some((nx, ny, nw, nh));
        self.bg_status = Some(format!("Resized selection to {nw}x{nh}, tiled as a pattern."));
    }

    /// Lunar Magic's "Add Offset to Background Tiles" (F9 on a selection):
    /// adds a signed offset to every non-empty tile in the selection (or the
    /// whole background when nothing is selected). The offset applies in
    /// absolute block space; the bank follows the first non-empty tile and
    /// stragglers are clamped to that bank, like Lunar Magic.
    fn bg_apply_offset(&mut self) {
        let offset = self.bg_offset_val;
        let tiles = match &self.layer2_background {
            Some(bg) => bg.read(|l| l.tile_ids.clone()),
            None => return,
        };
        let cells: Vec<(u32, u32)> = match self.bg_selection {
            Some((x, y, w, h)) => (0..h).flat_map(|r| (0..w).map(move |c| (x + c, y + r))).collect(),
            None => {
                (0..BG_TILEMAP_HEIGHT as u32).flat_map(|r| (0..BG_TILEMAP_WIDTH as u32).map(move |c| (c, r))).collect()
            }
        };
        let Some(result) = bg_tile_offset(&tiles, &cells, self.bg_page, offset) else {
            self.bg_status = Some(if offset == 0 {
                "Offset is 0; nothing to do.".to_string()
            } else {
                "No non-empty tiles in scope.".to_string()
            });
            return;
        };
        let old_page = self.bg_page;
        self.bg_page = result.new_page;
        if result.new_page != old_page {
            // The selector shows the current page's blocks.
            self.bg_selector_tex = None;
        }
        let n = result.edits.len();
        self.bg_commit_tiles(&result.edits);
        self.bg_status = Some(format!(
            "Offset {offset:+}: moved {n} tile{}.{bank}{clamped}",
            if n == 1 { "" } else { "s" },
            bank = if result.new_page != old_page {
                format!(" Background Map16 bank switched {old_page} -> {}.", result.new_page)
            } else {
                String::new()
            },
            clamped = if result.clamped > 0 {
                format!(
                    " {} tile{} clamped to the bank edge.",
                    result.clamped,
                    if result.clamped == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            },
        ));
    }

    /// Lunar Magic's "Remap Background Tiles": re-match every non-empty tile
    /// against the current bank's Map16 blocks by tile graphics, so a
    /// background keeps its look after "Change Background Map16 Bank".
    /// Tiles with no identical block in the new bank are left unchanged and
    /// reported.
    fn bg_remap_tiles(&mut self) {
        let page = self.bg_page as usize;
        // Source graphics: the bank the tiles were addressing before the last
        // bank switch, else the current bank (a no-op then, unless duplicate
        // blocks exist).
        let src_page = self.bg_prev_page.unwrap_or(self.bg_page) as usize;
        let tiles = match &self.layer2_background {
            Some(bg) => bg.read(|l| l.tile_ids.clone()),
            None => return,
        };
        let mut by_words = std::collections::HashMap::new();
        for b in 0..256usize {
            let w = self.bg_block_words.get(page * 256 + b).copied().unwrap_or([0; 4]);
            by_words.entry(w).or_insert(b as u8);
        }
        let mut edits = Vec::new();
        let (mut remapped, mut unmatched) = (0u32, 0u32);
        for (idx, &t) in tiles.iter().enumerate() {
            if t == 0 {
                continue;
            }
            let want = self.bg_block_words.get(src_page * 256 + t as usize).copied().unwrap_or([0; 4]);
            match by_words.get(&want) {
                Some(&b) => {
                    if b != t {
                        edits.push((idx, b));
                        remapped += 1;
                    }
                }
                None => unmatched += 1,
            }
        }
        // Idempotent: the tiles now name new-bank blocks, so a second run
        // matches them against the current bank and changes nothing.
        self.bg_prev_page = None;
        if edits.is_empty() && unmatched == 0 {
            self.bg_status = Some("All tiles already match the current bank.".to_string());
        } else {
            self.bg_commit_tiles(&edits);
            self.bg_status = Some(format!(
                "Remapped {remapped} tile{}. {unmatched} had no identical block in bank {page} and were left unchanged.",
                if remapped == 1 { "" } else { "s" },
            ));
        }
    }

    /// Lunar Magic's "Change Background Map16 Bank": switch the 256-block
    /// Map16 page the tilemap addresses. The tile IDs are untouched; on save
    /// the data is relocated across the $0CE8FE boundary so the game fills the
    /// matching high byte.
    fn bg_apply_bank(&mut self) {
        let new_page = self.bg_bank_choice.min(1);
        if new_page == self.bg_page {
            self.bg_status = Some(format!("Already on background Map16 bank {new_page}."));
            return;
        }
        let old = self.bg_page;
        self.bg_prev_page = Some(old);
        self.bg_page = new_page;
        self.bg_selector_tex = None;
        // The tilemap is unchanged, but the WRAM high bytes, canvas and dirty
        // flag must follow the new bank.
        self.bg_after_edit();
        self.bg_status = Some(format!(
            "Switched to background Map16 bank {new_page} (blocks ${:03X}-${:03X}). \
             On save the data moves to the matching side of $0CE8FE; use Remap Background Tiles to keep the same graphics.",
            new_page as u16 * 0x100,
            new_page as u16 * 0x100 + 0xFF,
        ));
    }

    /// Lunar Magic's "Copy Background Image": copy the WYSIWYG canvas to the
    /// system clipboard as a 512x432 image.
    fn bg_copy_image(&mut self) {
        let tiles = match &self.layer2_background {
            Some(bg) => bg.read(|l| l.tile_ids.clone()),
            None => return,
        };
        let pixels = self.bg_render_pixels(&tiles);
        let result = arboard::Clipboard::new().and_then(|mut cb| {
            cb.set_image(arboard::ImageData {
                width:  CANVAS_W as usize,
                height: CANVAS_H as usize,
                bytes:  pixels.into(),
            })
        });
        match result {
            Ok(()) => {
                self.bg_status = Some("Background image copied to clipboard (512x432).".to_string());
            }
            Err(e) => {
                self.bg_status = Some(format!("Could not copy image to clipboard: {e}"));
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Window
// -------------------------------------------------------------------------------------------------

/// Normalize two corner cells into (x, y, w, h), clamped to the tilemap.
fn normalize_rect(a: (u32, u32), b: (u32, u32)) -> (u32, u32, u32, u32) {
    let x = a.0.min(b.0).min(BG_TILEMAP_WIDTH as u32 - 1);
    let y = a.1.min(b.1).min(BG_TILEMAP_HEIGHT as u32 - 1);
    let w = (a.0.max(b.0) - x + 1).min(BG_TILEMAP_WIDTH as u32 - x);
    let h = (a.1.max(b.1) - y + 1).min(BG_TILEMAP_HEIGHT as u32 - y);
    (x, y, w, h)
}

/// Resize an (ox, oy, ow, oh) cell rect by dragging `handle` to cell
/// (`tile_x`, `tile_y`), clamped to the 32x27 tilemap and at least 1x1.
fn apply_bg_resize(
    ox: u32, oy: u32, ow: u32, oh: u32, handle: DragHandle, tile_x: i32, tile_y: i32,
) -> (u32, u32, u32, u32) {
    use DragHandle::*;
    let max_w = BG_TILEMAP_WIDTH as u32;
    let max_h = BG_TILEMAP_HEIGHT as u32;
    let (mut nx, mut ny, mut nw, mut nh) = (ox, oy, ow, oh);
    match handle {
        E | Ne | Se => {
            nw = (tile_x - ox as i32 + 1).clamp(1, max_w as i32 - ox as i32) as u32;
        }
        W | Nw | Sw => {
            let right = ox as i32 + ow as i32;
            let nl = tile_x.clamp(0, right - 1);
            nw = (right - nl) as u32;
            nx = nl as u32;
        }
        _ => {}
    }
    match handle {
        S | Se | Sw => {
            nh = (tile_y - oy as i32 + 1).clamp(1, max_h as i32 - oy as i32) as u32;
        }
        N | Ne | Nw => {
            let bottom = oy as i32 + oh as i32;
            let nt = tile_y.clamp(0, bottom - 1);
            nh = (bottom - nt) as u32;
            ny = nt as u32;
        }
        _ => {}
    }
    (nx, ny, nw, nh)
}

impl UiLevelEditor {
    pub(super) fn bg_tilemap_editor_window(&mut self, ctx: &Context) {
        if !self.show_bg_tilemap_editor {
            return;
        }
        if self.layer2_background.is_none() {
            self.show_bg_tilemap_editor = false;
            return;
        }
        if self.bg_canvas_dirty {
            self.bg_rebuild_canvas(ctx);
            self.bg_canvas_dirty = false;
        }
        if self.bg_selector_tex.is_none() {
            self.bg_rebuild_selector(ctx);
        }

        let mut open = self.show_bg_tilemap_editor;
        egui::Window::new("Background Tile Map Editor")
            .open(&mut open)
            .resizable(true)
            .default_size([920.0, 640.0])
            .show(ctx, |ui| {
                self.bg_editor_ui(ui);
            });
        self.show_bg_tilemap_editor = open;

        self.bg_offset_dialog(ctx);
        self.bg_bank_dialog(ctx);
    }

    fn bg_editor_ui(&mut self, ui: &mut Ui) {
        // ── Tool row ──
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.bg_tool, BgTileTool::Paint, "Paint");
            ui.selectable_value(&mut self.bg_tool, BgTileTool::Select, "Select");
            ui.selectable_value(&mut self.bg_tool, BgTileTool::Eyedropper, "Eyedropper");
            ui.separator();
            let can_undo = self.layer2_background.as_ref().map(|bg| bg.can_undo()).unwrap_or(false);
            let can_redo = self.layer2_background.as_ref().map(|bg| bg.can_redo()).unwrap_or(false);
            if ui.add_enabled(can_undo, egui::Button::new("Undo")).clicked() {
                self.bg_undo();
            }
            if ui.add_enabled(can_redo, egui::Button::new("Redo")).clicked() {
                self.bg_redo();
            }
            ui.separator();
            ui.selectable_value(&mut self.bg_zoom, 1.0, "1x");
            ui.selectable_value(&mut self.bg_zoom, 2.0, "2x");
            ui.checkbox(&mut self.bg_show_grid, "Grid");
            if ui.button("Select All").clicked() {
                self.bg_selection = Some((0, 0, BG_TILEMAP_WIDTH as u32, BG_TILEMAP_HEIGHT as u32));
                self.bg_status = Some("Selected the whole background.".to_string());
            }
        });
        ui.separator();

        // ── Canvas + side panel ──
        ui.horizontal(|ui| {
            egui::ScrollArea::both().id_salt("bg_tilemap_canvas_scroll").max_height(480.0).show(ui, |ui| {
                self.bg_canvas_ui(ui);
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("bg_tilemap_side_scroll").max_height(480.0).show(ui, |ui| {
                ui.set_min_width(272.0);
                self.bg_side_panel_ui(ui);
            });
        });

        if let Some(status) = self.bg_status.clone() {
            ui.separator();
            ui.label(status);
        }

        // ── Keyboard shortcuts (consumed here so the main canvas doesn't
        // also act on them while this window is open) ──
        let text_focused = ui.ctx().memory(|m| m.focused().is_some());
        if !text_focused {
            ui.input_mut(|input| {
                if input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)) {
                    self.bg_undo();
                } else if input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y)) {
                    self.bg_redo();
                }
                if input.key_pressed(egui::Key::F9) {
                    self.bg_offset_open = true;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) && self.bg_selection.is_some() {
                    self.bg_clear_selection();
                }
                if input.key_pressed(egui::Key::Escape) && self.bg_drag.is_some() {
                    self.bg_drag = None;
                    self.bg_canvas_dirty = true;
                    self.bg_status = Some("Drag cancelled.".to_string());
                }
            });
        }
    }

    /// Screen rect of a cell rect, in canvas pixels.
    fn bg_sel_rect_px(&self, rect: &Rect, sel: (u32, u32, u32, u32)) -> Rect {
        let (x, y, w, h) = sel;
        let z = self.bg_zoom;
        Rect::from_min_max(
            Pos2::new(rect.min.x + x as f32 * 16.0 * z, rect.min.y + y as f32 * 16.0 * z),
            Pos2::new(rect.min.x + (x + w) as f32 * 16.0 * z, rect.min.y + (y + h) as f32 * 16.0 * z),
        )
    }

    fn bg_canvas_ui(&mut self, ui: &mut Ui) {
        use egui::PointerButton::{Middle, Primary, Secondary};

        let zoom = self.bg_zoom;
        let canvas_px = Vec2::new(CANVAS_W as f32 * zoom, CANVAS_H as f32 * zoom);
        let (response, painter) = ui.allocate_painter(canvas_px, Sense::click_and_drag());
        let rect = response.rect;

        if let Some(tex) = self.bg_canvas_tex.clone() {
            painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }

        if self.bg_show_grid {
            let stroke = egui::Stroke::new(1.0_f32, Color32::from_black_alpha(70));
            for c in 0..=BG_TILEMAP_WIDTH as u32 {
                let x = rect.min.x + c as f32 * 16.0 * zoom;
                painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], stroke);
            }
            for r in 0..=BG_TILEMAP_HEIGHT as u32 {
                let y = rect.min.y + r as f32 * 16.0 * zoom;
                painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], stroke);
            }
        }

        // Live selection rect: the drag's rect while selecting/resizing.
        let live_sel: Option<(u32, u32, u32, u32)> = match &self.bg_drag {
            Some(BgDrag::Select { start, cur }) => Some(normalize_rect(*start, *cur)),
            Some(BgDrag::Resize { cur, .. }) => Some(*cur),
            _ => self.bg_selection,
        };
        if let Some(sel) = live_sel {
            let r = self.bg_sel_rect_px(&rect, sel);
            painter.rect_stroke(r, 0.0, egui::Stroke::new(2.0_f32, Color32::YELLOW), egui::StrokeKind::Outside);
            // Lunar Magic-style white resize handles.
            for (_, hr) in drag_handle_rects(r, 8.0) {
                painter.rect_filled(hr, 0.0, Color32::WHITE);
                painter.rect_stroke(hr, 0.0, egui::Stroke::new(1.0_f32, Color32::BLACK), egui::StrokeKind::Outside);
            }
        }

        // Pointer position -> tilemap cell.
        let cell_at = |pos: Pos2| -> Option<(u32, u32)> {
            let p = pos - rect.min;
            if p.x < 0.0 || p.y < 0.0 {
                return None;
            }
            let c = (p.x / (16.0 * zoom)) as u32;
            let r = (p.y / (16.0 * zoom)) as u32;
            (c < BG_TILEMAP_WIDTH as u32 && r < BG_TILEMAP_HEIGHT as u32).then_some((c, r))
        };

        // ── Drag start: resize handle first, then the active tool ──
        if response.drag_started_by(Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                if self.bg_drag.is_none() {
                    if let Some(sel) = self.bg_selection {
                        let r = self.bg_sel_rect_px(&rect, sel);
                        if let Some(handle) = handle_at(r, 8.0, pos) {
                            let (x, y, w, h) = sel;
                            let tiles = self
                                .layer2_background
                                .as_ref()
                                .map(|bg| bg.read(|l| l.tile_ids.clone()))
                                .unwrap_or_default();
                            let mut content = Vec::with_capacity((w * h) as usize);
                            for rr in 0..h {
                                for cc in 0..w {
                                    content.push(
                                        tiles[bg_cell_index(x + cc, y + rr).unwrap_or(0).min(BG_TILEMAP_LEN - 1)],
                                    );
                                }
                            }
                            self.bg_drag = Some(BgDrag::Resize { handle, orig: sel, cur: sel, content, cw: w, ch: h });
                        }
                    }
                }
                if self.bg_drag.is_none() {
                    match self.bg_tool {
                        BgTileTool::Paint => {
                            if let Some((c, r)) = cell_at(pos) {
                                self.bg_drag = Some(BgDrag::Paint { cells: vec![(c, r)] });
                                self.bg_canvas_dirty = true;
                            }
                        }
                        BgTileTool::Select => {
                            if let Some((c, r)) = cell_at(pos) {
                                self.bg_drag = Some(BgDrag::Select { start: (c, r), cur: (c, r) });
                            }
                        }
                        BgTileTool::Eyedropper => {
                            if let Some((c, r)) = cell_at(pos) {
                                self.bg_pick_tile(c, r);
                            }
                        }
                    }
                }
            }
        }

        // ── Drag continued: update the transient drag state ──
        if response.dragged_by(Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                match &mut self.bg_drag {
                    Some(BgDrag::Paint { cells }) => {
                        if let Some((c, r)) = cell_at(pos) {
                            if cells.last() != Some(&(c, r)) {
                                cells.push((c, r));
                                self.bg_canvas_dirty = true;
                            }
                        }
                    }
                    Some(BgDrag::Select { cur, .. }) => {
                        if let Some((c, r)) = cell_at(pos) {
                            *cur = (c, r);
                        }
                    }
                    Some(BgDrag::Resize { handle, orig, cur, .. }) => {
                        if let Some((c, r)) = cell_at(pos) {
                            *cur = apply_bg_resize(orig.0, orig.1, orig.2, orig.3, *handle, c as i32, r as i32);
                            self.bg_canvas_dirty = true;
                        }
                    }
                    None => {}
                }
            }
        }

        // ── Drag released: commit one undo step ──
        if response.drag_stopped_by(Primary) {
            match self.bg_drag.take() {
                Some(BgDrag::Paint { cells }) => {
                    let tile = self.bg_selected_tile;
                    let tiles =
                        self.layer2_background.as_ref().map(|bg| bg.read(|l| l.tile_ids.clone())).unwrap_or_default();
                    let mut seen = HashSet::new();
                    let mut edits = Vec::new();
                    for (c, r) in cells {
                        if let Some(idx) = bg_cell_index(c, r) {
                            if seen.insert(idx) && tiles[idx] != tile {
                                edits.push((idx, tile));
                            }
                        }
                    }
                    if edits.is_empty() {
                        // Clear the live preview.
                        self.bg_canvas_dirty = true;
                    } else {
                        let n = edits.len();
                        self.bg_commit_tiles(&edits);
                        self.bg_status = Some(format!("Painted {n} tile{}.", if n == 1 { "" } else { "s" }));
                    }
                }
                Some(BgDrag::Select { start, cur }) => {
                    let (x, y, w, h) = normalize_rect(start, cur);
                    self.bg_selection = Some((x, y, w, h));
                    self.bg_status = Some(format!("Selected {w}x{h} at ({x}, {y})."));
                }
                Some(BgDrag::Resize { orig, cur, content, cw, ch, .. }) => {
                    self.bg_commit_resize(orig, cur, &content, cw, ch);
                }
                None => {}
            }
        }

        // ── Right-click: erase; Shift+Right-click: pattern fill ──
        if response.clicked_by(Secondary) {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some((c, r)) = cell_at(pos) {
                    let shift = response.ctx.input(|i| i.modifiers.shift);
                    if shift {
                        self.bg_flood_fill(c, r);
                    } else if let Some(idx) = bg_cell_index(c, r) {
                        let cur = self.layer2_background.as_ref().map(|bg| bg.read(|l| l.tile_ids[idx]));
                        if cur != Some(0) {
                            self.bg_commit_tiles(&[(idx, 0)]);
                            self.bg_status = Some("Erased tile.".to_string());
                        }
                    }
                }
            }
        }

        // ── Middle-click: quick pick ──
        if response.clicked_by(Middle) {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some((c, r)) = cell_at(pos) {
                    self.bg_pick_tile(c, r);
                }
            }
        }
    }

    fn bg_side_panel_ui(&mut self, ui: &mut Ui) {
        ui.heading("Tile");
        if let Some(tex) = self.bg_selector_tex.clone() {
            let (resp, painter) = ui.allocate_painter(Vec2::new(256.0, 256.0), Sense::click());
            painter.image(tex.id(), resp.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let (sc, sr) = (self.bg_selected_tile % 16, self.bg_selected_tile / 16);
            let hr = Rect::from_min_max(
                Pos2::new(resp.rect.min.x + sc as f32 * SEL_CELL as f32, resp.rect.min.y + sr as f32 * SEL_CELL as f32),
                Pos2::new(
                    resp.rect.min.x + (sc + 1) as f32 * SEL_CELL as f32,
                    resp.rect.min.y + (sr + 1) as f32 * SEL_CELL as f32,
                ),
            );
            painter.rect_stroke(hr, 0.0, egui::Stroke::new(2.0_f32, Color32::YELLOW), egui::StrokeKind::Outside);
            if resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let p = pos - resp.rect.min;
                    let c = (p.x / SEL_CELL as f32) as u32;
                    let r = (p.y / SEL_CELL as f32) as u32;
                    if c < SEL_COLS && r < 16 {
                        self.bg_selected_tile = (r * SEL_COLS + c) as u8;
                    }
                }
            }
        }
        ui.label(format!("Selected block: ${:02X} (bank {})", self.bg_selected_tile, self.bg_page));
        ui.separator();

        ui.heading("Background");
        ui.label(format!(
            "Map16 bank: {} (blocks ${:03X}-${:03X})",
            self.bg_page,
            self.bg_page as u16 * 0x100,
            self.bg_page as u16 * 0x100 + 0xFF,
        ));
        if ui.button("Change Background Map16 Bank…").clicked() {
            self.bg_bank_choice = self.bg_page;
            self.bg_bank_open = true;
        }
        if ui.button("Remap Background Tiles").clicked() {
            self.bg_remap_tiles();
        }
        if ui.button("Copy Background Image").clicked() {
            self.bg_copy_image();
        }
        if ui.button("Add Offset to Background Tiles…").clicked() {
            self.bg_offset_open = true;
        }
        ui.separator();
        ui.label(
            "Paint: left-drag. Erase: right-click. Pattern fill: Shift+right-click \
             (fills with the selection as a pattern, or the selected tile). \
             Pick: middle-click or the eyedropper. Resize the selection as a \
             pattern by dragging its white handles. F9 adds an offset to tiles.",
        )
        .on_hover_text("Matches Lunar Magic's Background Tile Map Editor gestures.");
    }

    fn bg_offset_dialog(&mut self, ctx: &Context) {
        if !self.bg_offset_open {
            return;
        }
        let mut open = self.bg_offset_open;
        let mut val = self.bg_offset_val;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Add Offset to Background Tiles").open(&mut open).collapsible(false).resizable(false).show(
            ctx,
            |ui| {
                ui.label("Adds an offset to every non-empty background tile.");
                ui.label("Scope: the selection, or the whole background when nothing is selected.");
                ui.label(
                    "Tiles that cross $FF/$00 automatically switch the background Map16 bank, \
                     like Lunar Magic's F9.",
                );
                ui.horizontal(|ui| {
                    ui.label("Offset:");
                    ui.add(egui::DragValue::new(&mut val).range(-255..=255));
                });
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if cancel {
            open = false;
        }
        self.bg_offset_open = open;
        self.bg_offset_val = val;
        if apply {
            self.bg_offset_open = false;
            self.bg_apply_offset();
        }
    }

    fn bg_bank_dialog(&mut self, ctx: &Context) {
        if !self.bg_bank_open {
            return;
        }
        let mut open = self.bg_bank_open;
        let mut choice = self.bg_bank_choice;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Change Background Map16 Bank").open(&mut open).collapsible(false).resizable(false).show(
            ctx,
            |ui| {
                ui.label("Selects which 256-block Map16 page the background tiles address.");
                ui.radio_value(&mut choice, 0, "Bank 0 — blocks $00–$FF (pointer below $0CE8FE)");
                ui.radio_value(&mut choice, 1, "Bank 1 — blocks $100–$1FF (pointer at/above $0CE8FE)");
                ui.label(
                    "On save the background data is moved to the matching side of $0CE8FE. \
                     If that side has no free space, saving fails with an explicit error \
                     instead of writing to the wrong side.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if cancel {
            open = false;
        }
        self.bg_bank_open = open;
        self.bg_bank_choice = choice;
        if apply {
            self.bg_bank_open = false;
            self.bg_apply_bank();
        }
    }
}
