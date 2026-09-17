use egui::{ColorImage, Context, DragValue, Slider};
use smwe_rom::{
    title_credits::{self, ENEMY_NAME_COUNT, ENEMY_NAME_LABELS},
    title_stripe::{
        encode_credits_stripe,
        encode_player_select_stripe,
        parse_title_stripe,
        split_credits_commands,
        split_player_select_commands,
        TitleStripeCommand,
        TitleTileGrid,
        CREDITS_L3_FIRST_ROW,
        CREDITS_L3_LAST_ROW,
        MENU_FIRST_ROW,
        MENU_LAST_ROW,
        TITLE_TILEMAP_BLANK,
        TITLE_TILEMAP_HEIGHT,
        TITLE_TILEMAP_VRAM_BASE,
        TITLE_TILEMAP_WIDTH,
    },
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

                ui.heading("Title screen full tilemap (WYSIWYG)");
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
                    // Budget meters: logo stripe and player-select menu stripe.
                    let logo_used = self.title_credits.title_screen_stripe.len();
                    let logo_max = title_credits::TITLE_SCREEN_STRIPE_MAX_SIZE;
                    ui.horizontal(|ui| {
                        ui.label(format!("Logo stripe: {logo_used} / {logo_max} bytes"));
                        let frac = logo_used as f32 / logo_max as f32;
                        ui.add(egui::ProgressBar::new(frac).desired_width(200.0));
                    });
                    let menu_used = self.title_credits.player_select_stripe.len();
                    let menu_max = title_credits::PLAYER_SELECT_STRIPE_MAX_SIZE;
                    ui.horizontal(|ui| {
                        ui.label(format!("Menu stripe: {menu_used} / {menu_max} bytes"));
                        let frac = menu_used as f32 / menu_max as f32;
                        ui.add(egui::ProgressBar::new(frac).desired_width(200.0));
                    });
                    ui.label(format!(
                        "Rows {MENU_FIRST_ROW}–{MENU_LAST_ROW} paint to the menu stripe; all other rows paint to the logo stripe."
                    ));
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
                        // Ensure the grid and GFX are loaded.
                        self.ensure_credits_gfx();
                        self.ensure_credits_grid();
                        if let Some(err) = self.credits_grid_error.clone() {
                            ui.colored_label(egui::Color32::from_rgb(220, 80, 70), err);
                        }
                        // Budget meter.
                        let used = self.title_credits.enemy_name_stripes[i].len();
                        ui.horizontal(|ui| {
                            ui.label(format!("Stripe: {used} / {slot_size} bytes"));
                            let frac = used as f32 / slot_size as f32;
                            ui.add(egui::ProgressBar::new(frac).desired_width(200.0));
                        });
                        ui.label(format!(
                            "Rows {CREDITS_L3_FIRST_ROW}–{CREDITS_L3_LAST_ROW} are editable Layer-3 text (full viewable area)."
                        ));
                        // WYSIWYG grid preview.
                        if self.credits_grid_tex.is_none() {
                            if let Some(img) = self.render_credits_grid_image() {
                                let tex = ctx.load_texture(
                                    "credits_grid",
                                    img,
                                    egui::TextureOptions::NEAREST,
                                );
                                self.credits_grid_tex = Some(tex);
                            }
                        }
                        if let Some(tex) = self.credits_grid_tex.clone() {
                            let (w, h) = (TITLE_TILEMAP_WIDTH as f32 * 8.0, (CREDITS_L3_LAST_ROW + 1) as f32 * 8.0);
                            // Scale to fit (max 512px wide).
                            let scale = (512.0 / w).min(1.0);
                            let (dw, dh) = (w * scale, h * scale);
                            let (resp, painter) = ui.allocate_painter(
                                egui::Vec2::new(dw, dh),
                                egui::Sense::click_and_drag(),
                            );
                            painter.image(
                                tex.id(),
                                resp.rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            // Paint interaction.
                            if resp.dragged() || resp.clicked() {
                                if let Some(pos) = resp.interact_pointer_pos() {
                                    let rel = pos - resp.rect.min;
                                    let gx = (rel.x / scale / 8.0) as usize;
                                    let gy = (rel.y / scale / 8.0) as usize;
                                    if gx < TITLE_TILEMAP_WIDTH && gy <= CREDITS_L3_LAST_ROW {
                                        // Paint with the current brush (tile word).
                                        // For credits, use a simple text tile: tile 0x0F ('A') with palette 6.
                                        // TODO: Add a proper tile picker for credits.
                                        let word = 0x3800 | (self.title_paint_word & 0x3FF);
                                        self.paint_credits_cell(gx, gy, word);
                                    }
                                }
                            }
                            // Right-click to pick.
                            if resp.secondary_clicked() {
                                if let Some(pos) = resp.interact_pointer_pos() {
                                    let rel = pos - resp.rect.min;
                                    let gx = (rel.x / scale / 8.0) as usize;
                                    let gy = (rel.y / scale / 8.0) as usize;
                                    if gx < TITLE_TILEMAP_WIDTH && gy <= CREDITS_L3_LAST_ROW {
                                        if let Some(grid) = self.credits_grid.as_ref() {
                                            self.title_paint_word = grid.cells[gy][gx];
                                        }
                                    }
                                }
                            }
                        }
                        // Tile word editor for the paint brush.
                        ui.horizontal(|ui| {
                            ui.label("Paint tile:");
                            let mut word = self.title_paint_word as i32;
                            if ui.add(DragValue::new(&mut word).range(0..=0xFFFF).hexadecimal(4, false, true)).changed() {
                                self.title_paint_word = word as u16;
                            }
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
    ///
    /// The display grid composites the logo stripe and the player-select menu
    /// stripe (menu draws after the logo, exactly like `LoadScrnImage` runs
    /// them on hardware). Edits are routed by row: rows
    /// `MENU_FIRST_ROW..=MENU_LAST_ROW` belong to the menu stripe, all other
    /// rows belong to the logo stripe.
    fn ensure_title_grid(&mut self) {
        let logo_len = self.title_credits.title_screen_stripe.len();
        let menu_len = self.title_credits.player_select_stripe.len();
        if self.title_grid.is_some()
            && self.title_grid_for_stripe_len == Some(logo_len)
            && self.menu_grid_for_stripe_len == Some(menu_len)
        {
            return;
        }
        // Parse the logo stripe.
        let logo_grid = match TitleTileGrid::from_stripe(&self.title_credits.title_screen_stripe) {
            Ok(grid) => grid,
            Err(e) => {
                self.title_grid = None;
                self.title_grid_error = Some(format!("Cannot parse title stripe: {e}"));
                return;
            }
        };
        // Parse the player-select menu stripe: preserved RLE clears plus an
        // editable text grid.
        let (menu_grid, menu_clears) = match parse_title_stripe(&self.title_credits.player_select_stripe) {
            Ok(cmds) => {
                let (clears, text) = split_player_select_commands(&cmds);
                (TitleTileGrid::from_commands(&text), clears)
            }
            Err(e) => {
                self.title_grid = None;
                self.title_grid_error = Some(format!("Cannot parse player select stripe: {e}"));
                return;
            }
        };
        // Composite: logo, then menu clears, then menu text (hardware order).
        // Start from the logo grid and apply every menu command in order.
        let mut display = logo_grid.clone();
        let mut all_menu_cmds = menu_clears.clone();
        // Rebuild text commands from the menu grid for compositing.
        for y in MENU_FIRST_ROW..=MENU_LAST_ROW {
            let row = &menu_grid.cells[y];
            if let (Some(x0), Some(x1)) = (
                row.iter().position(|&w| w != TITLE_TILEMAP_BLANK),
                row.iter().rposition(|&w| w != TITLE_TILEMAP_BLANK),
            ) {
                let tiles = row[x0..=x1].to_vec();
                let nbytes = tiles.len() * 2;
                all_menu_cmds.push(TitleStripeCommand {
                    vram_dest: TITLE_TILEMAP_VRAM_BASE + (y * TITLE_TILEMAP_WIDTH + x0) as u16,
                    vertical: false,
                    rle: false,
                    nbytes,
                    tiles,
                });
            }
        }
        for cmd in &all_menu_cmds {
            let mut dest = cmd.vram_dest as usize;
            let stride = if cmd.vertical { 32 } else { 1 };
            for &tile in &cmd.expanded_tiles() {
                if dest >= TITLE_TILEMAP_VRAM_BASE as usize {
                    let wo = dest - TITLE_TILEMAP_VRAM_BASE as usize;
                    if wo < TITLE_TILEMAP_WIDTH * TITLE_TILEMAP_HEIGHT {
                        display.cells[wo / TITLE_TILEMAP_WIDTH][wo % TITLE_TILEMAP_WIDTH] = tile;
                    }
                }
                dest += stride;
            }
        }
        self.title_grid = Some(display);
        self.menu_grid = Some(menu_grid);
        self.menu_clears = menu_clears;
        self.title_grid_for_stripe_len = Some(logo_len);
        self.menu_grid_for_stripe_len = Some(menu_len);
        self.title_grid_error = None;
        self.title_grid_tex = None; // re-render
    }

    /// Capture the title screen's tile graphics (VRAM $4000 word base) and
    /// palette (CGRAM) by running the real title init on a scratch CPU clone.
    /// This mirrors `GM04PrepTitleScreen`: level 0xEB init plus the title
    /// palette overrides. Cached; the graphics don't change while editing.
    ///
    /// The title screen runs in BG Mode 1, so Layer 3 is 2bpp: VRAM words
    /// $4000-$5FFF hold 512 tiles ($000-$1FF), 16 bytes each.
    fn ensure_title_graphics(&mut self) {
        if self.title_grid_vram.is_some() && self.title_grid_cgram.is_some() {
            return;
        }
        let mut scratch = self.cpu.clone();
        smwe_emu::emu::decompress_sublevel(&mut scratch, 0xEB);
        smwe_emu::emu::load_title_screen_palette(&mut scratch);
        // VRAM is byte-addressed; word $4000 -> byte $8000. The 2bpp L3
        // character data spans words $4000-$5FFF (8 KiB = 512 tiles).
        self.title_grid_vram = Some(scratch.mem.vram[0x8000..0xA000].to_vec());
        self.title_grid_cgram = Some(scratch.mem.cgram.to_vec());
    }

    /// Rasterize the 64×64 title grid to a 512×512 image using the captured
    /// VRAM/CGRAM, exactly as the PPU would draw the Layer 3 tilemap in BG
    /// Mode 1 (2bpp tiles, 8 palettes of 4 colors).
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
                // 2bpp: 512 tiles max ($000-$1FF).
                if tile >= 512 {
                    continue;
                }
                let pal = ((word >> 10) & 0x7) as usize;
                let flip_x = word & 0x4000 != 0;
                let flip_y = word & 0x8000 != 0;
                // 2bpp tile: 16 bytes, byte pair per row.
                let tile_base = tile * 16;
                for py in 0..8 {
                    for px in 0..8 {
                        let sx = if flip_x { 7 - px } else { px };
                        let sy = if flip_y { 7 - py } else { py };
                        let row_off = tile_base + sy * 2;
                        if row_off + 1 >= vram.len() {
                            continue;
                        }
                        let b0 = vram[row_off];
                        let b1 = vram[row_off + 1];
                        let bit = 7 - sx;
                        let c0 = (b0 >> bit) & 1;
                        let c1 = (b1 >> bit) & 1;
                        let ci = (c0 | (c1 << 1)) as usize;
                        if ci == 0 {
                            continue;
                        }
                        let rgb = read_color(pal * 4 + ci);
                        img[(tx * 8 + px, ty * 8 + py)] = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                    }
                }
            }
        }
        Some(img)
    }

    /// Paint one grid cell, re-encode the owning stripe, and commit it. Rows
    /// `MENU_FIRST_ROW..=MENU_LAST_ROW` belong to the player-select menu
    /// stripe; all other rows belong to the title logo stripe. On over-budget
    /// the edit is refused and the grid left untouched.
    fn paint_title_cell(&mut self, x: usize, y: usize, word: u16) {
        if y >= MENU_FIRST_ROW && y <= MENU_LAST_ROW {
            self.paint_menu_cell(x, y, word);
        } else {
            self.paint_logo_cell(x, y, word);
        }
    }

    /// Paint a logo-stripe cell.
    fn paint_logo_cell(&mut self, x: usize, y: usize, word: u16) {
        // Re-parse the logo grid from the current stripe bytes (it is the
        // source of truth; the display grid is a composite).
        let mut logo_grid = match TitleTileGrid::from_stripe(&self.title_credits.title_screen_stripe) {
            Ok(g) => g,
            Err(_) => return,
        };
        if logo_grid.cells[y][x] == word {
            return;
        }
        logo_grid.cells[y][x] = word;
        match logo_grid.to_stripe_bytes() {
            Ok(bytes) => {
                self.title_credits.title_screen_stripe = bytes;
                self.title_grid_for_stripe_len = Some(self.title_credits.title_screen_stripe.len());
                self.title_grid_tex = None; // re-render
                self.title_grid_error = None;
                self.title_credits_dirty = true;
                self.has_edits = true;
                // Invalidate the composite display grid.
                self.title_grid = None;
            }
            Err(e) => {
                self.title_grid_error = Some(format!("{e}"));
            }
        }
    }

    /// Paint a menu-stripe cell (rows `MENU_FIRST_ROW..=MENU_LAST_ROW`).
    fn paint_menu_cell(&mut self, x: usize, y: usize, word: u16) {
        let menu_grid = match self.menu_grid.as_mut() {
            Some(g) => g,
            None => return,
        };
        if menu_grid.cells[y][x] == word {
            return;
        }
        let old = menu_grid.cells[y][x];
        menu_grid.cells[y][x] = word;
        match encode_player_select_stripe(&self.menu_clears, menu_grid) {
            Ok(bytes) => {
                self.title_credits.player_select_stripe = bytes;
                self.menu_grid_for_stripe_len = Some(self.title_credits.player_select_stripe.len());
                self.title_grid_tex = None; // re-render
                self.title_grid_error = None;
                self.title_credits_dirty = true;
                self.has_edits = true;
                // Invalidate the composite display grid.
                self.title_grid = None;
            }
            Err(e) => {
                menu_grid.cells[y][x] = old; // refuse: restore
                self.title_grid_error = Some(format!("{e}"));
            }
        }
    }

    // -------------------------------------------------------------------------------------------------
    // Credits WYSIWYG editor (Lunar Magic v3.40 parity: full viewable area).
    // -------------------------------------------------------------------------------------------------

    /// Ensure the GFX2F letter tiles are loaded (for credits text rendering).
    fn ensure_credits_gfx(&mut self) {
        if self.credits_gfx2f_tiles.is_some() {
            return;
        }
        // GFX2F (file 0x2F) contains the 2bpp credits font.
        if let Some(file) = self.rom.gfx.files.get(0x2F) {
            self.credits_gfx2f_tiles = Some(file.tiles.clone());
        }
    }

    /// Ensure the credits grid is parsed for the selected scene.
    fn ensure_credits_grid(&mut self) {
        let scene = self.credits_editor_selected.min(ENEMY_NAME_COUNT - 1);
        if self.credits_grid.is_some() && self.credits_grid_for_scene == Some(scene) {
            return;
        }
        let stripe = &self.title_credits.enemy_name_stripes[scene];
        match parse_title_stripe(stripe) {
            Ok(cmds) => {
                let (non_l3, l3) = split_credits_commands(&cmds);
                let grid = TitleTileGrid::from_commands(&l3);
                self.credits_grid = Some(grid);
                self.credits_non_l3 = non_l3;
                self.credits_grid_for_scene = Some(scene);
                self.credits_grid_error = None;
                self.credits_grid_tex = None;
            }
            Err(e) => {
                self.credits_grid = None;
                self.credits_grid_error = Some(format!("Cannot parse credits stripe: {e}"));
            }
        }
    }

    /// Paint one credits grid cell (only rows CREDITS_L3_FIRST_ROW..=CREDITS_L3_LAST_ROW
    /// are editable). Re-encodes the stripe; on over-budget the edit is refused.
    fn paint_credits_cell(&mut self, x: usize, y: usize, word: u16) {
        if y < CREDITS_L3_FIRST_ROW || y > CREDITS_L3_LAST_ROW {
            return;
        }
        let scene = self.credits_editor_selected.min(ENEMY_NAME_COUNT - 1);
        let slot_size = title_credits::TitleCreditsData::enemy_name_slot_size(scene);
        let grid = match self.credits_grid.as_mut() {
            Some(g) => g,
            None => return,
        };
        if grid.cells[y][x] == word {
            return;
        }
        let old = grid.cells[y][x];
        grid.cells[y][x] = word;
        match encode_credits_stripe(&self.credits_non_l3, grid, slot_size) {
            Ok(bytes) => {
                self.title_credits.enemy_name_stripes[scene] = bytes;
                self.credits_grid_tex = None;
                self.credits_grid_error = None;
                self.title_credits_dirty = true;
                self.has_edits = true;
            }
            Err(e) => {
                grid.cells[y][x] = old; // refuse: restore
                self.credits_grid_error = Some(format!("{e}"));
            }
        }
    }

    /// Render the credits L3 grid using GFX2F letter tiles. The tile number
    /// from the word indexes directly into GFX2F (2bpp, 64 tiles). Uses a
    /// simple white-on-transparent palette for the preview; the exact
    /// in-game colors come from the credits palette.
    fn render_credits_grid_image(&self) -> Option<ColorImage> {
        let grid = self.credits_grid.as_ref()?;
        let tiles = self.credits_gfx2f_tiles.as_ref()?;
        let (w, h) = (TITLE_TILEMAP_WIDTH * 8, (CREDITS_L3_LAST_ROW + 1) * 8);
        let mut img = ColorImage::new([w, h], egui::Color32::from_rgb(0, 0, 0));
        for y in CREDITS_L3_FIRST_ROW..=CREDITS_L3_LAST_ROW {
            for x in 0..TITLE_TILEMAP_WIDTH {
                let word = grid.cells[y][x];
                if word == TITLE_TILEMAP_BLANK {
                    continue;
                }
                let tile_idx = (word & 0x3FF) as usize;
                if tile_idx >= tiles.len() {
                    continue;
                }
                let tile = &tiles[tile_idx];
                let flip_x = word & 0x4000 != 0;
                let flip_y = word & 0x8000 != 0;
                for py in 0..8 {
                    for px in 0..8 {
                        // GFX2F tiles are stored mirrored; flip by default.
                        let sx = if flip_x { px } else { 7 - px };
                        let sy = if flip_y { py } else { 7 - py };
                        let ci = tile.color_indices[sy * 8 + sx];
                        if ci == 0 {
                            continue;
                        }
                        // White text for preview.
                        img[(x * 8 + px, y * 8 + py)] = egui::Color32::WHITE;
                    }
                }
            }
        }
        Some(img)
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
