use egui::{ColorImage, Context, DragValue, Slider};
use smwe_rom::{
    title_credits::{self, ENEMY_NAME_COUNT, ENEMY_NAME_LABELS},
    title_stripe::{TitleTileGrid, TITLE_TILEMAP_BLANK, TITLE_TILEMAP_HEIGHT, TITLE_TILEMAP_WIDTH},
};

use super::UiLevelEditor;

/// Global editor for title-screen and ending enemy-name data that lives in
/// fixed vanilla ROM slots.
impl UiLevelEditor {
    pub(super) fn title_credits_editor_window(&mut self, ctx: &Context) {
        if !self.show_title_credits_editor {
            return;
        }

        let mut open = self.show_title_credits_editor;
        egui::Window::new("Title Screen / Credits").open(&mut open).resizable(true).default_size([620.0, 520.0]).show(
            ctx,
            |ui| {
                ui.label("Edits here are global and use vanilla fixed-size data slots.");
                ui.separator();

                ui.heading("Title screen");
                ui.horizontal(|ui| {
                    ui.label("Opening overworld submap:");
                    let mut submap = self.title_credits.title_submap as i32;
                    if ui.add(Slider::new(&mut submap, 0..=6).hexadecimal(1, false, false)).changed() {
                        self.title_credits.title_submap = submap as u8;
                        self.title_credits_dirty = true;
                        self.has_edits = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Demo input: {} / {} bytes",
                        self.title_credits.title_demo_inputs.len() * 2 + 1,
                        title_credits::TITLE_INPUT_SEQ_MAX_SIZE
                    ));
                    if ui.button("+ Step").clicked()
                        && self.title_credits.title_demo_inputs.len() * 2 + 3 <= title_credits::TITLE_INPUT_SEQ_MAX_SIZE
                    {
                        self.title_credits
                            .title_demo_inputs
                            .push(title_credits::TitleDemoInput { buttons: 0x00, duration: 0x10 });
                        self.title_credits_dirty = true;
                        self.has_edits = true;
                    }
                    if ui.button("- Step").clicked() && !self.title_credits.title_demo_inputs.is_empty() {
                        self.title_credits.title_demo_inputs.pop();
                        self.title_credits_dirty = true;
                        self.has_edits = true;
                    }
                });

                egui::ScrollArea::vertical().max_height(150.0).id_salt("title_demo_inputs").show(ui, |ui| {
                    egui::Grid::new("title_demo_input_grid").num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label("#");
                        ui.label("Buttons");
                        ui.label("Duration");
                        ui.label("Held");
                        ui.end_row();
                        for (i, input) in self.title_credits.title_demo_inputs.iter_mut().enumerate() {
                            ui.label(format!("{i:02}"));
                            let mut buttons = input.buttons as i32;
                            if ui
                                .add(DragValue::new(&mut buttons).range(0..=0xFF).hexadecimal(2, false, false))
                                .changed()
                            {
                                input.buttons = buttons as u8;
                                self.title_credits_dirty = true;
                                self.has_edits = true;
                            }
                            let mut duration = input.duration as i32;
                            if ui
                                .add(DragValue::new(&mut duration).range(0..=0xFF).hexadecimal(2, false, false))
                                .changed()
                            {
                                input.duration = duration as u8;
                                self.title_credits_dirty = true;
                                self.has_edits = true;
                            }
                            ui.label(button_summary(input.buttons));
                            ui.end_row();
                        }
                    });
                });

                ui.heading("Title logo / menu (WYSIWYG)");
                self.ensure_title_grid();
                self.ensure_title_graphics();
                if let Some(err) = &self.title_grid_error {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 70), err.clone());
                }
                if self.title_grid.is_some() {
                    // Render the preview texture on demand.
                    if self.title_grid_tex.is_none() {
                        if let Some(img) = self.render_title_grid_image() {
                            self.title_grid_tex =
                                Some(ctx.load_texture("title_grid", img, egui::TextureOptions::NEAREST));
                        }
                    }
                    if let Some(tex) = &self.title_grid_tex {
                        // Show the 64x64 tilemap (512x512 px) scaled to fit;
                        // click/drag paints the selected tile word.
                        let avail = ui.available_width().min(512.0);
                        let scale = avail / 512.0;
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(avail, avail), egui::Sense::drag());
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        // Highlight the selected cell.
                        if let Some((sx, sy)) = self.title_selected_cell {
                            let cell = egui::Rect::from_min_size(
                                rect.min + egui::vec2(sx as f32 * 8.0 * scale, sy as f32 * 8.0 * scale),
                                egui::vec2(8.0 * scale, 8.0 * scale),
                            );
                            ui.painter().rect_stroke(
                                cell,
                                0.0,
                                egui::Stroke::new(2.0f32, egui::Color32::YELLOW),
                                egui::StrokeKind::Outside,
                            );
                        }
                        // Paint on click/drag.
                        if response.dragged() || response.clicked() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                let lx = ((pos.x - rect.min.x) / scale / 8.0) as usize;
                                let ly = ((pos.y - rect.min.y) / scale / 8.0) as usize;
                                if lx < TITLE_TILEMAP_WIDTH && ly < TITLE_TILEMAP_HEIGHT {
                                    self.title_selected_cell = Some((lx, ly));
                                    let word = self.title_paint_word;
                                    self.paint_title_cell(lx, ly, word);
                                }
                            }
                        }
                        // Right-click picks the tile word under the cursor.
                        if response.secondary_clicked() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                let lx = ((pos.x - rect.min.x) / scale / 8.0) as usize;
                                let ly = ((pos.y - rect.min.y) / scale / 8.0) as usize;
                                if lx < TITLE_TILEMAP_WIDTH && ly < TITLE_TILEMAP_HEIGHT {
                                    if let Some(grid) = &self.title_grid {
                                        self.title_paint_word = grid.cells[ly][lx];
                                        self.title_selected_cell = Some((lx, ly));
                                    }
                                }
                            }
                        }
                    }
                    // Budget meter.
                    let used = self.title_credits.title_screen_stripe.len();
                    let max = title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE;
                    ui.horizontal(|ui| {
                        ui.label(format!("Stripe: {used} / {max} bytes"));
                        let frac = used as f32 / max as f32;
                        ui.add(egui::ProgressBar::new(frac).desired_width(200.0));
                    });
                    // Tile word editor for the paint brush / selected cell.
                    ui.horizontal(|ui| {
                        ui.label("Paint tile:");
                        let mut word = self.title_paint_word as i32;
                        if ui.add(DragValue::new(&mut word).range(0..=0xFFFF).hexadecimal(4, false, false)).changed() {
                            self.title_paint_word = word as u16;
                        }
                        let tile = (self.title_paint_word & 0x3FF) as i32;
                        let pal = ((self.title_paint_word >> 10) & 0x7) as i32;
                        ui.label(format!("tile ${tile:03X} pal {pal}"));
                        if ui.button("Erase").clicked() {
                            self.title_paint_word = TITLE_TILEMAP_BLANK;
                        }
                        if ui.button("Pick selected").clicked() {
                            if let Some((sx, sy)) = self.title_selected_cell {
                                if let Some(grid) = &self.title_grid {
                                    self.title_paint_word = grid.cells[sy][sx];
                                }
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut tile = (self.title_paint_word & 0x3FF) as i32;
                        let mut pal = ((self.title_paint_word >> 10) & 0x7) as i32;
                        let mut prio = ((self.title_paint_word >> 13) & 1) != 0;
                        let mut fx = ((self.title_paint_word >> 14) & 1) != 0;
                        let mut fy = ((self.title_paint_word >> 15) & 1) != 0;
                        if ui
                            .add(
                                DragValue::new(&mut tile).range(0..=0x3FF).hexadecimal(3, false, false).prefix("tile "),
                            )
                            .changed()
                        {
                            self.title_paint_word = (self.title_paint_word & !0x3FF) | (tile as u16 & 0x3FF);
                        }
                        if ui.add(DragValue::new(&mut pal).range(0..=7).prefix("pal ")).changed() {
                            self.title_paint_word = (self.title_paint_word & !(0x7 << 10)) | ((pal as u16 & 0x7) << 10);
                        }
                        if ui.checkbox(&mut prio, "prio").changed() {
                            self.title_paint_word = (self.title_paint_word & !(1 << 13)) | ((prio as u16) << 13);
                        }
                        if ui.checkbox(&mut fx, "flipX").changed() {
                            self.title_paint_word = (self.title_paint_word & !(1 << 14)) | ((fx as u16) << 14);
                        }
                        if ui.checkbox(&mut fy, "flipY").changed() {
                            self.title_paint_word = (self.title_paint_word & !(1 << 15)) | ((fy as u16) << 15);
                        }
                    });
                    ui.label("Left-click/drag: paint · Right-click: pick tile · Erase paints the blank tile.");
                }
                ui.collapsing("Raw title stripe bytes (advanced)", |ui| {
                    egui::ScrollArea::vertical().max_height(160.0).id_salt("title_stripe_bytes").show(ui, |ui| {
                        egui::Grid::new("title_stripe_byte_grid").num_columns(8).spacing([4.0, 4.0]).show(ui, |ui| {
                            for (byte_i, byte) in self.title_credits.title_screen_stripe.iter_mut().enumerate() {
                                let mut v = *byte as i32;
                                if ui.add(DragValue::new(&mut v).range(0..=0xFF).hexadecimal(2, false, false)).changed()
                                {
                                    *byte = v as u8;
                                    self.title_credits_dirty = true;
                                    self.has_edits = true;
                                    // Invalidate the WYSIWYG grid cache.
                                    self.title_grid_for_stripe_len = None;
                                }
                                if byte_i % 8 == 7 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                    if !self.title_credits.title_screen_stripe.ends_with(&[0xFF]) {
                        ui.colored_label(egui::Color32::from_rgb(220, 80, 70), "Title stripe must end with FF.");
                    }
                });

                ui.separator();
                ui.heading("Ending enemy-name stripes");
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().max_height(240.0).id_salt("credits_enemy_list").show(ui, |ui| {
                            for i in 0..ENEMY_NAME_COUNT {
                                let used = self.title_credits.enemy_name_stripes[i].len();
                                let max = title_credits::TitleCreditsData::enemy_name_slot_size(i);
                                ui.selectable_value(
                                    &mut self.credits_editor_selected,
                                    i,
                                    format!("{i:02X} {} ({used}/{max} B)", ENEMY_NAME_LABELS[i]),
                                );
                            }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let i = self.credits_editor_selected.min(ENEMY_NAME_COUNT - 1);
                        let slot_size = title_credits::TitleCreditsData::enemy_name_slot_size(i);
                        let summary =
                            title_credits::summarize_enemy_name_stripe(&self.title_credits.enemy_name_stripes[i]);
                        if summary.is_empty() {
                            ui.label("Decoded text: (none detected)");
                        } else {
                            ui.label(format!("Decoded text: {summary}"));
                        }
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "Raw bytes: {} / {slot_size}",
                                self.title_credits.enemy_name_stripes[i].len()
                            ));
                            if ui.button("+ Byte").clicked()
                                && self.title_credits.enemy_name_stripes[i].len() < slot_size
                            {
                                let insert_at = self.title_credits.enemy_name_stripes[i].len().saturating_sub(1);
                                self.title_credits.enemy_name_stripes[i].insert(insert_at, 0xFC);
                                self.title_credits_dirty = true;
                                self.has_edits = true;
                            }
                            if ui.button("- Byte").clicked() && self.title_credits.enemy_name_stripes[i].len() > 1 {
                                let remove_at = self.title_credits.enemy_name_stripes[i].len() - 2;
                                self.title_credits.enemy_name_stripes[i].remove(remove_at);
                                self.title_credits_dirty = true;
                                self.has_edits = true;
                            }
                        });
                        if !self.title_credits.enemy_name_stripes[i].ends_with(&[0xFF]) {
                            ui.colored_label(egui::Color32::from_rgb(220, 80, 70), "Stripe must end with FF.");
                        }
                        egui::ScrollArea::vertical().max_height(220.0).id_salt("credits_enemy_bytes").show(ui, |ui| {
                            egui::Grid::new("credits_enemy_byte_grid").num_columns(8).spacing([4.0, 4.0]).show(
                                ui,
                                |ui| {
                                    for (byte_i, byte) in
                                        self.title_credits.enemy_name_stripes[i].iter_mut().enumerate()
                                    {
                                        let mut v = *byte as i32;
                                        if ui
                                            .add(DragValue::new(&mut v).range(0..=0xFF).hexadecimal(2, false, false))
                                            .changed()
                                        {
                                            *byte = v as u8;
                                            self.title_credits_dirty = true;
                                            self.has_edits = true;
                                        }
                                        if byte_i % 8 == 7 {
                                            ui.end_row();
                                        }
                                    }
                                },
                            );
                        });
                    });
                });
            },
        );

        self.show_title_credits_editor = open;
    }

    /// Ensure the title grid is parsed from the current stripe bytes,
    /// invalidating the cache when the bytes changed (e.g. via the raw
    /// byte editor below).
    fn ensure_title_grid(&mut self) {
        let len = self.title_credits.title_screen_stripe.len();
        if self.title_grid.is_some() && self.title_grid_for_stripe_len == Some(len) {
            return;
        }
        match TitleTileGrid::from_stripe(&self.title_credits.title_screen_stripe) {
            Ok(grid) => {
                self.title_grid = Some(grid);
                self.title_grid_for_stripe_len = Some(len);
                self.title_grid_error = None;
                self.title_grid_tex = None; // re-render
            }
            Err(e) => {
                self.title_grid = None;
                self.title_grid_error = Some(format!("Cannot parse title stripe: {e}"));
            }
        }
    }

    /// Capture the title screen's tile graphics (VRAM $4000 word base) and
    /// palette (CGRAM) by running the real title init on a scratch CPU clone.
    /// This mirrors `GM04PrepTitleScreen`: level 0xEB init plus the title
    /// palette overrides. Cached; the graphics don't change while editing.
    fn ensure_title_graphics(&mut self) {
        if self.title_grid_vram.is_some() && self.title_grid_cgram.is_some() {
            return;
        }
        let mut scratch = self.cpu.clone();
        smwe_emu::emu::decompress_sublevel(&mut scratch, 0xEB);
        smwe_emu::emu::load_title_screen_palette(&mut scratch);
        // VRAM is byte-addressed; word $4000 -> byte $8000. The 4bpp L3
        // character data spans words $4000-$4FFF (8 KiB).
        self.title_grid_vram = Some(scratch.mem.vram[0x8000..0xA000].to_vec());
        self.title_grid_cgram = Some(scratch.mem.cgram.to_vec());
    }

    /// Rasterize the 64×64 title grid to a 512×512 image using the captured
    /// VRAM/CGRAM, exactly as the PPU would draw the Layer 3 tilemap.
    fn render_title_grid_image(&self) -> Option<ColorImage> {
        let grid = self.title_grid.as_ref()?;
        let vram = self.title_grid_vram.as_ref()?;
        let cgram = self.title_grid_cgram.as_ref()?;
        let read_color = |idx: usize| -> [u8; 3] {
            let off = idx * 2;
            if off + 1 >= cgram.len() {
                return [0, 0, 0];
            }
            let rgb = cgram[off] as u16 | ((cgram[off + 1] as u16) << 8);
            [((rgb & 0x1F) << 3) as u8, (((rgb >> 5) & 0x1F) << 3) as u8, (((rgb >> 10) & 0x1F) << 3) as u8]
        };
        // SNES backdrop color (CGRAM 0) behind transparent pixels.
        let backdrop = read_color(0);
        let (w, h) = (TITLE_TILEMAP_WIDTH * 8, TITLE_TILEMAP_HEIGHT * 8);
        let mut img = ColorImage::new([w, h], egui::Color32::from_rgb(backdrop[0], backdrop[1], backdrop[2]));
        for (ty, row) in grid.cells.iter().enumerate() {
            for (tx, &word) in row.iter().enumerate() {
                let tile = (word & 0x3FF) as usize;
                let pal = ((word >> 10) & 0x7) as usize;
                let flip_x = word & 0x4000 != 0;
                let flip_y = word & 0x8000 != 0;
                let tile_base = tile * 32;
                for py in 0..8 {
                    for px in 0..8 {
                        let sx = if flip_x { 7 - px } else { px };
                        let sy = if flip_y { 7 - py } else { py };
                        let row_off = tile_base + sy * 2;
                        if row_off + 17 >= vram.len() {
                            continue;
                        }
                        let b0 = vram[row_off];
                        let b1 = vram[row_off + 1];
                        let b2 = vram[row_off + 16];
                        let b3 = vram[row_off + 17];
                        let bit = 7 - sx;
                        let c0 = (b0 >> bit) & 1;
                        let c1 = (b1 >> bit) & 1;
                        let c2 = (b2 >> bit) & 1;
                        let c3 = (b3 >> bit) & 1;
                        let ci = (c0 | (c1 << 1) | (c2 << 2) | (c3 << 3)) as usize;
                        if ci == 0 {
                            continue;
                        }
                        let rgb = read_color(pal * 16 + ci);
                        img[(tx * 8 + px, ty * 8 + py)] = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                    }
                }
            }
        }
        Some(img)
    }

    /// Paint one grid cell, re-encode the stripe, and commit it. On
    /// over-budget the edit is refused and the grid left untouched.
    fn paint_title_cell(&mut self, x: usize, y: usize, word: u16) {
        let grid = match self.title_grid.as_mut() {
            Some(g) => g,
            None => return,
        };
        if grid.cells[y][x] == word {
            return;
        }
        let old = grid.cells[y][x];
        grid.cells[y][x] = word;
        match grid.to_stripe_bytes() {
            Ok(bytes) => {
                self.title_credits.title_screen_stripe = bytes;
                self.title_grid_for_stripe_len = Some(self.title_credits.title_screen_stripe.len());
                self.title_grid_tex = None; // re-render
                self.title_grid_error = None;
                self.title_credits_dirty = true;
                self.has_edits = true;
            }
            Err(e) => {
                grid.cells[y][x] = old; // refuse: restore
                self.title_grid_error = Some(format!("{e}"));
            }
        }
    }
}

fn button_summary(buttons: u8) -> String {
    let mut names = Vec::new();
    for (mask, name) in [
        (0x80, "B"),
        (0x40, "Y"),
        (0x20, "Select"),
        (0x10, "Start"),
        (0x08, "Up"),
        (0x04, "Down"),
        (0x02, "Left"),
        (0x01, "Right"),
    ] {
        if buttons & mask != 0 {
            names.push(name);
        }
    }
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join("+")
    }
}
