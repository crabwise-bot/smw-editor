use std::collections::BTreeMap;

use egui::{vec2, Color32, Context, Rect, Sense, Slider, Vec2};
use egui_phosphor::regular as icon;

use super::{tile_picker::render_sub_tile, UiLevelEditor};
use crate::{ui::tool::DockableEditorTool, undo::Undo};

const PREVIEW_PX: usize = 32; // display size for each 8x8 sub-tile preview

/// The Map16 editor's per-block tile-word edits. Wrapped in
/// [`UndoableData`] for Lunar Magic v1.91-style Ctrl+Z / Ctrl+Y undo/redo.
/// Serialized sorted-by-key (`BTreeMap` iteration order) as
/// `block_id:u16 + 4×tile_word:u16` records (10 bytes each), so undo deltas
/// are deterministic — a `HashMap`'s order is not.
#[derive(Clone, Debug, Default)]
pub(super) struct EditableMap16Edits {
    pub edits: BTreeMap<u16, [u16; 4]>,
}

impl Undo for EditableMap16Edits {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut edits = BTreeMap::new();
        for chunk in bytes.chunks_exact(10) {
            let block_id = u16::from_le_bytes([chunk[0], chunk[1]]);
            let mut words = [0u16; 4];
            for (i, w) in words.iter_mut().enumerate() {
                *w = u16::from_le_bytes([chunk[2 + i * 2], chunk[3 + i * 2]]);
            }
            edits.insert(block_id, words);
        }
        Self { edits }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.edits.len() * 10);
        for (&block_id, words) in &self.edits {
            bytes.extend_from_slice(&block_id.to_le_bytes());
            for &w in words {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        self.edits.len() * 10
    }
}

impl UiLevelEditor {
    pub(super) fn map16_editor_window(&mut self, ctx: &Context) {
        if !self.show_map16_editor {
            return;
        }
        let mut open = self.show_map16_editor;
        let win = egui::Window::new("Map16 Block Editor").open(&mut open).resizable(false).show(ctx, |ui| {
            // Block selector (full FG range: vanilla 0x000-0x1FF plus LM
            // expanded pages 0x02-0x7F).
            ui.horizontal(|ui| {
                ui.label("Block:");
                let mut bid = self.selected_map16_block_for_edit.unwrap_or(self.draw_block_id);
                if ui.add(Slider::new(&mut bid, 0..=0x7FFF).hexadecimal(4, false, true)).changed() {
                    self.selected_map16_block_for_edit = Some(bid);
                }
                if ui.small_button("Use draw block").clicked() {
                    self.selected_map16_block_for_edit = Some(self.draw_block_id);
                }
            });

            let block_id = self.selected_map16_block_for_edit.unwrap_or(self.draw_block_id);
            self.ensure_map16_block_ptr(block_id);

            // Get current tile words (from edits or ROM)
            let mut tile_words = self.get_block_tile_words(block_id);

            // "Acts like" (act-as): the per-FG-tile gameplay reference,
            // Lunar Magic 1.91+ parity. BG tiles have no act-as value.
            if block_id < 0x8000 {
                ui.horizontal(|ui| {
                    ui.label("Acts like:");
                    let mut act = self.act_as_of(block_id) as i32;
                    if ui.add(Slider::new(&mut act, 0..=0x7FFF).hexadecimal(4, false, true)).changed() {
                        self.map16_acts_edits.insert(block_id, act as u16);
                        self.mark_edited();
                    }
                    if ui.small_button("Reset").clicked() {
                        let rom_act = smwe_rom::map16_expanded::act_as_in_rom(self.rom.rom_bytes(), 0, block_id);
                        if rom_act == block_id {
                            self.map16_acts_edits.remove(&block_id);
                        } else {
                            // ROM has a stored non-identity value: write identity back.
                            self.map16_acts_edits.insert(block_id, block_id);
                        }
                        self.mark_edited();
                    }
                    if ui.small_button("Remap…").clicked() {
                        self.map16_remap_open = true;
                        self.map16_remap_preview = None;
                    }
                });
                ui.small("Gameplay values are < 0x200 (LM enforces this in-game). Blank = acts as itself.");
            } else {
                ui.small("BG tiles have no acts-like value.");
            }
            // Vanilla behavior-category reference (hardcoded ID ranges, not
            // the act-as table).
            let category = smwe_rom::block_behavior::category_of(block_id);
            ui.horizontal(|ui| {
                ui.label("Behavior:");
                ui.strong(category.label());
            });
            if let Some(behavior) = smwe_rom::block_behavior::specific_behavior(block_id) {
                ui.small(format!("Specific: {behavior}"));
            }

            ui.separator();

            // Labels for sub-tile positions
            let sub_labels = ["Upper Left", "Lower Left", "Upper Right", "Lower Right"];
            let mut changed = false;

            for (sub_i, label) in sub_labels.iter().enumerate() {
                ui.group(|ui| {
                    ui.label(*label);
                    let t = tile_words[sub_i];

                    // Render sub-tile preview
                    let mut pixels = vec![0u8; PREVIEW_PX * PREVIEW_PX * 4];
                    // Fill checkerboard background for transparency
                    for y in 0..PREVIEW_PX {
                        for x in 0..PREVIEW_PX {
                            let off = (y * PREVIEW_PX + x) * 4;
                            let checker = ((x / 4 + y / 4) % 2 == 0) as u8;
                            let shade = if checker == 0 { 64u8 } else { 96u8 };
                            pixels[off] = shade;
                            pixels[off + 1] = shade;
                            pixels[off + 2] = shade;
                            pixels[off + 3] = 255;
                        }
                    }
                    // Scale factor: PREVIEW_PX / 8 = 4
                    let scale = PREVIEW_PX / 8;
                    let mut raw_pixels = vec![0u8; 8 * 8 * 4];
                    render_sub_tile(&self.cpu.mem.vram, &self.cpu.mem.cgram, t, 0, 0, &mut raw_pixels, 8);
                    // Upscale into preview
                    for sy in 0..8usize {
                        for sx in 0..8usize {
                            let src = (sy * 8 + sx) * 4;
                            if raw_pixels[src + 3] > 0 {
                                for dy in 0..scale {
                                    for dx in 0..scale {
                                        let dst = ((sy * scale + dy) * PREVIEW_PX + sx * scale + dx) * 4;
                                        pixels[dst] = raw_pixels[src];
                                        pixels[dst + 1] = raw_pixels[src + 1];
                                        pixels[dst + 2] = raw_pixels[src + 2];
                                        pixels[dst + 3] = 255;
                                    }
                                }
                            }
                        }
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied([PREVIEW_PX, PREVIEW_PX], &pixels);
                    let tex = ui.ctx().load_texture(
                        format!("map16_sub_{block_id}_{sub_i}_{t}"),
                        image,
                        egui::TextureOptions::NEAREST,
                    );
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(PREVIEW_PX as f32), Sense::click());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_size(egui::pos2(0., 0.), vec2(1., 1.)),
                        Color32::WHITE,
                    );
                    if response.double_clicked() {
                        self.open_tile_editor_from_map16(block_id, sub_i, t);
                    }
                    response.on_hover_text("Double-click to edit this 8×8 tile's pixels");

                    // Tile word fields
                    let mut tile_num = (t & 0x3FF) as i32;
                    let mut palette = ((t >> 10) & 0x7) as i32;
                    let mut flip_x = (t & 0x4000) != 0;
                    let mut flip_y = (t & 0x8000) != 0;
                    let mut priority = (t & 0x2000) != 0;
                    let mut sub_changed = false;

                    ui.horizontal(|ui| {
                        ui.label("Tile:");
                        sub_changed |=
                            ui.add(Slider::new(&mut tile_num, 0..=0x3FF).hexadecimal(3, false, true)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Palette:");
                        sub_changed |= ui.add(Slider::new(&mut palette, 0..=7)).changed();
                    });
                    ui.horizontal(|ui| {
                        sub_changed |= ui.checkbox(&mut flip_x, "Flip X").changed();
                        sub_changed |= ui.checkbox(&mut flip_y, "Flip Y").changed();
                        sub_changed |= ui.checkbox(&mut priority, "Priority").changed();
                    });
                    ui.monospace(format!("Word: {:04X}", t));

                    if sub_changed {
                        let new_t = (tile_num as u16 & 0x3FF)
                            | ((palette as u16 & 0x7) << 10)
                            | (if priority { 0x2000 } else { 0 })
                            | (if flip_x { 0x4000 } else { 0 })
                            | (if flip_y { 0x8000 } else { 0 });
                        tile_words[sub_i] = new_t;
                        changed = true;
                    }
                });
                if sub_i == 1 {
                    ui.separator();
                }
            }

            if changed {
                // Gesture-style edit: snapshot once, mutate directly; a
                // single undo step is committed when the gesture ends (see
                // the end of this function), so one slider drag is one undo.
                if self.map16_gesture_before.is_none() {
                    self.map16_gesture_before = Some(self.map16_edits.read(|e| e.clone()));
                }
                self.map16_edits.data_mut().edits.insert(block_id, tile_words);
                self.mark_edited();
            }

            // ── Undo/redo (Lunar Magic v1.91 added Ctrl+Z/Ctrl+Y to the ───
            // Map16 editor).
            ui.separator();
            ui.horizontal(|ui| {
                let can_undo = self.map16_edits.can_undo();
                if ui
                    .add_enabled(can_undo, egui::Button::new(format!("{} Undo", icon::ARROW_COUNTER_CLOCKWISE)))
                    .on_hover_text("Undo Map16 change (Ctrl+Z)")
                    .clicked()
                {
                    self.map16_undo();
                }
                let can_redo = self.map16_edits.can_redo();
                if ui
                    .add_enabled(can_redo, egui::Button::new(format!("{} Redo", icon::ARROW_CLOCKWISE)))
                    .on_hover_text("Redo Map16 change (Ctrl+Y)")
                    .clicked()
                {
                    self.map16_redo();
                }
            });

            // Revert button
            if self.map16_edits.read(|e| e.edits.contains_key(&block_id))
                || self.map16_acts_edits.contains_key(&block_id)
            {
                ui.separator();
                if ui.button("Revert to ROM").clicked() {
                    self.map16_edits.write(|e| {
                        e.edits.remove(&block_id);
                    });
                    self.map16_acts_edits.remove(&block_id);
                    self.mark_edited();
                }
            }

            // ── Clipboard: copy/paste this block's tile words ─────────────
            // Lunar Magic v1.63 has clipboard copy/paste in the 16x16 editor;
            // the payload is `smwclip:1:` text, so the four hex tile words
            // can also be read straight out of a paste into any text field.
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("⧉ Copy block").on_hover_text("Copy this block's 4 tile words (Ctrl+C)").clicked() {
                    crate::ui::clipboard::copy_payload(ctx, &crate::ui::clipboard::ClipboardPayload::Map16BlockWords {
                        words: tile_words,
                    });
                    self.mwl_status = Some(format!("Copied Map16 block {block_id:#06X} to clipboard"));
                }
                if ui.button("⎘ Paste block").on_hover_text("Paste tile words from the clipboard (Ctrl+V)").clicked()
                {
                    // The integration answers with Event::Paste on the next
                    // frame; picked up below (Ctrl+V arrives directly).
                    crate::ui::clipboard::request_paste(ctx);
                }
            });
            ui.small("Tip: Ctrl+C / Ctrl+V work here too when the window is focused.");

            self.map16_file_controls(ui);
        });
        self.show_map16_editor = open;
        self.map16_window_hovered = win.is_some_and(|r| r.response.hovered());

        // Ctrl+C while the pointer is over this window (the level canvas's
        // own clipboard keys stand down — see central_panel). Paste arrives
        // as Event::Paste directly on Ctrl+V, or after the Paste button's
        // viewport request — drain it here while this window has copy intent.
        if self.map16_window_hovered && !ctx.memory(|m| m.focused().is_some()) {
            if ctx.input(|i| i.events.contains(&egui::Event::Copy)) {
                let block_id = self.selected_map16_block_for_edit.unwrap_or(self.draw_block_id);
                let words = self.get_block_tile_words(block_id);
                crate::ui::clipboard::copy_payload(ctx, &crate::ui::clipboard::ClipboardPayload::Map16BlockWords {
                    words,
                });
                self.mwl_status = Some(format!("Copied Map16 block {block_id:#06X} to clipboard"));
            }
            if let Some(text) = crate::ui::clipboard::take_paste_text(ctx) {
                self.map16_apply_pasted_payload(&text);
            }
        }

        // ── Ctrl+Z / Ctrl+Y (Lunar Magic v1.91) while the pointer is over ──
        // this window (or while a tile-word drag is in flight). The window is
        // drawn before the central panel, so consuming here wins over the
        // level-canvas undo.
        let map16_active = self.map16_window_hovered || self.map16_gesture_before.is_some();
        if map16_active {
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)))
            {
                self.map16_undo();
            }
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y)))
            {
                self.map16_redo();
            }
        }

        // ── Commit the undo step when a slider-drag gesture ends ──────────
        // (snapshot taken on the first change; sliders fire changed() on
        // every drag frame, so committing per-frame would make undo walk
        // back through every intermediate value).
        if self.map16_gesture_before.is_some() && ctx.input(|i| i.pointer.any_released()) {
            self.commit_map16_gesture();
        }
    }

    /// Undo one Map16 edit (Lunar Magic v1.91 Map16-editor undo).
    /// An in-flight slider drag is committed first, so Ctrl+Z mid-drag
    /// undoes the drag rather than the edit before it.
    fn map16_undo(&mut self) {
        self.commit_map16_gesture();
        if self.map16_edits.can_undo() {
            self.map16_edits.undo();
            self.mark_edited();
        }
    }

    /// Redo one Map16 edit (Lunar Magic v1.91 Map16-editor redo).
    fn map16_redo(&mut self) {
        self.commit_map16_gesture();
        if self.map16_edits.can_redo() {
            self.map16_edits.redo();
            self.mark_edited();
        }
    }

    /// Commit an in-flight slider-drag gesture as a single undo step, if any.
    fn commit_map16_gesture(&mut self) {
        if let Some(before) = self.map16_gesture_before.take() {
            self.map16_edits.commit_change(&before);
        }
    }

    /// Apply a pasted clipboard payload in the Map16 Block Editor: tile words
    /// replace the current block's words; a single copied block ID jumps the
    /// editor to that block.
    fn map16_apply_pasted_payload(&mut self, text: &str) {
        let block_id = self.selected_map16_block_for_edit.unwrap_or(self.draw_block_id);
        match crate::ui::clipboard::ClipboardPayload::decode(text) {
            Some(crate::ui::clipboard::ClipboardPayload::Map16BlockWords { words }) => {
                self.map16_edits.write(|e| {
                    e.edits.insert(block_id, words);
                });
                self.mark_edited();
                self.mwl_status = Some(format!("Pasted tile words into Map16 block {block_id:#06X}"));
            }
            Some(crate::ui::clipboard::ClipboardPayload::Map16Blocks { cols: 1, rows: 1, ids }) => {
                let id = ids[0];
                self.selected_map16_block_for_edit = Some(id);
                self.mwl_status = Some(format!("Jumped to Map16 block {id:#06X}"));
            }
            Some(_) => {
                self.mwl_status = Some(
                    "Clipboard holds level/8x8/overworld data — nothing to paste into the Map16 editor".to_string(),
                );
            }
            None => {
                self.mwl_status = Some("Clipboard doesn't hold smw-editor data".to_string());
            }
        }
    }

    /// Effective "acts like" value for an FG tile: pending edit, else the
    /// ROM's act-as table, else identity. BG tiles have no act-as value.
    pub(super) fn act_as_of(&self, block_id: u16) -> u16 {
        if block_id >= 0x8000 {
            return block_id;
        }
        if let Some(&a) = self.map16_acts_edits.get(&block_id) {
            return a;
        }
        smwe_rom::map16_expanded::act_as_in_rom(self.rom.rom_bytes(), 0, block_id)
    }

    /// The act-as table as the user currently sees it: ROM table with
    /// pending edits overlaid.
    fn effective_acts_table(&self) -> std::collections::HashMap<u16, u16> {
        let mut table = smwe_rom::map16_expanded::read_acts_table(self.rom.rom_bytes(), 0).unwrap_or_default();
        for (&t, &a) in &self.map16_acts_edits {
            if a == t {
                table.remove(&t);
            } else {
                table.insert(t, a);
            }
        }
        table
    }

    pub(super) fn get_block_tile_words(&self, block_id: u16) -> [u16; 4] {
        if let Some(words) = self.map16_edits.read(|e| e.edits.get(&block_id).copied()) {
            return words;
        }
        if let Some(&snes_addr) = self.map16_block_ptrs.get(block_id as usize) {
            if snes_addr != 0 {
                use smwe_rom::snes_utils::addr::{AddrPc, AddrSnes};
                let rom_bytes = self.rom.rom_bytes();
                if let Ok(pc) = AddrPc::try_from_lorom(AddrSnes(snes_addr)) {
                    let base = pc.as_index();
                    let mut words = [0u16; 4];
                    for (i, w) in words.iter_mut().enumerate() {
                        let off = base + i * 2;
                        if off + 1 < rom_bytes.len() {
                            *w = rom_bytes[off] as u16 | ((rom_bytes[off + 1] as u16) << 8);
                        }
                    }
                    return words;
                }
            }
        }
        // Expanded FG blocks without a resolved pointer: read the page
        // through the expanded-page model (LM table, then the editor's
        // RATS block).
        if (0x200..0x8000).contains(&block_id) {
            if let Ok(Some(page)) =
                smwe_rom::map16_expanded::read_expanded_fg_page(self.rom.rom_bytes(), 0, (block_id >> 8) as u8)
            {
                let base = (block_id as usize & 0xFF) * 8;
                let mut words = [0u16; 4];
                for (i, w) in words.iter_mut().enumerate() {
                    let off = base + i * 2;
                    if off + 1 < page.len() {
                        *w = page[off] as u16 | ((page[off + 1] as u16) << 8);
                    }
                }
                return words;
            }
        }
        [0u16; 4]
    }
}

impl UiLevelEditor {
    /// Lunar Magic "Remap…" dialog (v1.91 / v3.01 parity). Two operations:
    /// - G: remap act-as *references* — `G100-101,+25` shifts every act-as
    ///   value in 0x100-0x101 by +0x25; `G100-101,M125` remaps them onto
    ///   base 0x125 (relative).
    /// - R: assign a tile range act-as values from a base — `R200-211,S25`
    ///   makes tiles 0x200-0x211 act as 0x25-0x36.
    /// Source values refer to pre-remap values; LM does not rewrite
    /// references automatically when blocks move, and neither does this.
    pub(super) fn map16_remap_window(&mut self, ctx: &Context) {
        if !self.map16_remap_open {
            return;
        }
        let mut open = self.map16_remap_open;
        egui::Window::new("Remap act-as values").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("Which tiles' act-as values point where. Values are hex; ranges like 100-1F3.");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.map16_remap_mode_g, true, "Remap references (G)");
                ui.radio_value(&mut self.map16_remap_mode_g, false, "Assign range from base (R)");
            });
            if self.map16_remap_mode_g {
                ui.horizontal(|ui| {
                    ui.label("Source range:");
                    ui.text_edit_singleline(&mut self.map16_remap_src);
                });
                ui.horizontal(|ui| {
                    ui.label("Reference:");
                    ui.text_edit_singleline(&mut self.map16_remap_ref);
                });
                ui.small("G100-101,+25 shifts by +0x25 · G100-101,M125 (or 125) remaps onto 0x125.");
            } else {
                ui.horizontal(|ui| {
                    ui.label("Tile range:");
                    ui.text_edit_singleline(&mut self.map16_remap_src);
                });
                ui.horizontal(|ui| {
                    ui.label("Base:");
                    ui.text_edit_singleline(&mut self.map16_remap_ref);
                });
                ui.small("R200-211,S25 (or 25): tiles 0x200-0x211 act as 0x25-0x36.");
            }
            ui.horizontal(|ui| {
                if ui.button("Preview").clicked() {
                    self.map16_remap_preview = Some(self.preview_remap());
                }
                if ui.button("Apply").clicked() {
                    match self.apply_remap() {
                        Ok(msg) => {
                            log::info!("{msg}");
                            self.map16_file_status = Some(msg);
                            self.map16_remap_open = false;
                        }
                        Err(e) => self.map16_remap_preview = Some(format!("Error: {e:#}")),
                    }
                }
            });
            if let Some(p) = self.map16_remap_preview.clone() {
                ui.separator();
                ui.monospace(p);
            }
            ui.small(
                "Note: in-game use of non-identity act-as values needs runtime support \
                 (Lunar Magic's expanded-Map16 ASM); the editor preserves and remaps the table.",
            );
        });
        self.map16_remap_open = open;
    }

    /// Compute the remap result against the effective table without writing.
    fn preview_remap(&self) -> String {
        match self.compute_remap() {
            Ok((before, after)) => {
                let mut changes: Vec<(u16, u16, u16)> = Vec::new();
                for (&t, &a_before) in &before {
                    let a_after = after.get(&t).copied().unwrap_or(t);
                    if a_before != a_after {
                        changes.push((t, a_before, a_after));
                    }
                }
                for (&t, &a_after) in &after {
                    if !before.contains_key(&t) && t != a_after {
                        changes.push((t, t, a_after));
                    }
                }
                changes.sort_unstable();
                if changes.is_empty() {
                    return "No tiles would change.".to_string();
                }
                let mut out = format!("{} tile(s) would change:\n", changes.len());
                for (t, b, a) in changes.iter().take(12) {
                    out.push_str(&format!("  {t:04X}: {b:04X} → {a:04X}\n"));
                }
                if changes.len() > 12 {
                    out.push_str(&format!("  … and {} more", changes.len() - 12));
                }
                out
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }

    /// Apply the remap dialog operation to the ROM file, preserving pending
    /// block/act-as edits by flushing them first.
    fn apply_remap(&mut self) -> anyhow::Result<String> {
        let (before, after) = self.compute_remap()?;
        let changed = after.iter().filter(|(&t, &a)| before.get(&t).copied().unwrap_or(t) != a).count()
            + before.iter().filter(|(&t, &a)| !after.contains_key(&t) && t != a).count();
        let mut rom_bytes = std::fs::read(&self.rom_path)?;
        let header_offset = super::mwl::smc_header_offset(&rom_bytes);
        // Flush pending edits first so the reload below doesn't drop them.
        self.save_to_rom(&mut rom_bytes, header_offset != 0)?;
        smwe_rom::map16_expanded::write_acts_table(&mut rom_bytes, header_offset, &after)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        super::mwl::write_rom_file_atomic(&self.rom_path, &rom_bytes)?;
        let fresh = smwe_rom::SmwRom::from_file(&self.rom_path)?;
        self.rom = std::sync::Arc::new(fresh);
        self.map16_edits = crate::undo::UndoableData::new(EditableMap16Edits::default());
        self.map16_acts_edits.clear();
        self.load_level();
        self.has_edits = false;
        Ok(format!("Remapped act-as values ({changed} tile(s) changed)"))
    }

    /// Parse the dialog fields and run the operation on a copy of the
    /// effective act-as table. Returns (before, after).
    fn compute_remap(
        &self,
    ) -> anyhow::Result<(std::collections::HashMap<u16, u16>, std::collections::HashMap<u16, u16>)> {
        use smwe_rom::map16_expanded::{remap_act_refs, remap_act_refs_delta, set_act_range_from_base};

        let before = self.effective_acts_table();
        let mut after = before.clone();
        if self.map16_remap_mode_g {
            let (s0, s1) = parse_hex_range(&self.map16_remap_src)
                .ok_or_else(|| anyhow::anyhow!("bad source range '{}'", self.map16_remap_src))?;
            if s0 >= 0x8000 || s1 >= 0x8000 {
                anyhow::bail!("source range must be FG act-as values (< 0x8000)");
            }
            let r = self.map16_remap_ref.trim();
            if let Some(d) = r.strip_prefix('+') {
                let delta = parse_hex_u16(d).ok_or_else(|| anyhow::anyhow!("bad delta '{d}'"))? as i32;
                remap_act_refs_delta(&mut after, s0, s1, delta);
            } else if let Some(d) = r.strip_prefix('-') {
                let delta = parse_hex_u16(d).ok_or_else(|| anyhow::anyhow!("bad delta '{d}'"))? as i32;
                remap_act_refs_delta(&mut after, s0, s1, -delta);
            } else {
                let base = parse_hex_u16(r.trim_start_matches(['M', 'm']))
                    .ok_or_else(|| anyhow::anyhow!("bad reference '{}'", self.map16_remap_ref))?;
                if base >= 0x8000 {
                    anyhow::bail!("reference must be < 0x8000");
                }
                remap_act_refs(&mut after, s0, s1, base);
            }
        } else {
            let (s0, s1) = parse_hex_range(&self.map16_remap_src)
                .ok_or_else(|| anyhow::anyhow!("bad tile range '{}'", self.map16_remap_src))?;
            if s1 >= 0x8000 {
                anyhow::bail!("tile range must be FG tiles (< 0x8000)");
            }
            let base = parse_hex_u16(self.map16_remap_ref.trim().trim_start_matches(['S', 's']))
                .ok_or_else(|| anyhow::anyhow!("bad base '{}'", self.map16_remap_ref))?;
            if base >= 0x8000 {
                anyhow::bail!("base must be < 0x8000");
            }
            set_act_range_from_base(&mut after, s0, s1, base);
        }
        Ok((before, after))
    }
}

/// Parse a hex u16, tolerating an optional `0x` prefix.
fn parse_hex_u16(s: &str) -> Option<u16> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.is_empty() {
        return None;
    }
    u16::from_str_radix(s, 16).ok()
}

/// Parse `A-B` or a single `A` as an inclusive hex range, tolerating a
/// leading `G`/`R` (so `G100-101` pastes straight from LM docs).
fn parse_hex_range(s: &str) -> Option<(u16, u16)> {
    let s = s.trim().trim_start_matches(['G', 'R', 'g', 'r']);
    if let Some((a, b)) = s.split_once('-') {
        let (a, b) = (parse_hex_u16(a)?, parse_hex_u16(b)?);
        Some((a.min(b), a.max(b)))
    } else {
        let a = parse_hex_u16(s)?;
        Some((a, a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::UndoableData;

    #[test]
    fn map16_undo_redo_round_trip() {
        let mut edits = UndoableData::new(EditableMap16Edits::default());
        edits.write(|e| {
            e.edits.insert(0x101, [0x0123, 0x4567, 0x89AB, 0xCDEF]);
        });
        edits.write(|e| {
            e.edits.insert(0x102, [0x0001, 0x0002, 0x0003, 0x0004]);
        });

        edits.undo();
        assert!(edits.read(|e| e.edits.get(&0x102).is_none()));
        assert_eq!(edits.read(|e| e.edits[&0x101]), [0x0123, 0x4567, 0x89AB, 0xCDEF]);

        // "Revert to ROM" removes the entry; undo restores it.
        edits.write(|e| {
            e.edits.remove(&0x101);
        });
        assert!(edits.read(|e| e.edits.get(&0x101).is_none()));
        edits.undo();
        assert_eq!(edits.read(|e| e.edits[&0x101]), [0x0123, 0x4567, 0x89AB, 0xCDEF]);

        edits.redo();
        assert!(edits.read(|e| e.edits.get(&0x101).is_none()));
    }

    #[test]
    fn map16_serialization_is_deterministic() {
        // Insertion order must not affect the serialized bytes: the undo
        // delta XORs byte sequences, so nondeterministic order would corrupt
        // undo/redo.
        let mut a = EditableMap16Edits::default();
        a.edits.insert(0x200, [1, 2, 3, 4]);
        a.edits.insert(0x101, [5, 6, 7, 8]);
        let mut b = EditableMap16Edits::default();
        b.edits.insert(0x101, [5, 6, 7, 8]);
        b.edits.insert(0x200, [1, 2, 3, 4]);
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_eq!(a.to_bytes().len(), 20);
        let back = EditableMap16Edits::from_bytes(a.to_bytes());
        assert_eq!(back.to_bytes(), a.to_bytes());
    }

    #[test]
    fn map16_gesture_commit_is_single_step() {
        let mut edits = UndoableData::new(EditableMap16Edits::default());
        // Slider-drag path: snapshot, mutate directly across frames, commit.
        let before = edits.read(|e| e.clone());
        for tile in [0x0100u16, 0x0101, 0x0102] {
            edits.data_mut().edits.insert(0x101, [tile, 0, 0, 0]);
        }
        edits.commit_change(&before);
        edits.undo();
        assert!(edits.read(|e| e.edits.get(&0x101).is_none()));
        assert!(!edits.can_undo());
    }
}
