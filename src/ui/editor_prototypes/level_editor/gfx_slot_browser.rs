//! Per-level GFX slot browser — Lunar Magic-style FG1/FG2/FG3/BG1 + SP1/SP2/SP3/SP4
//! slot list, cross-linked to the 8x8 tile editor.
//!
//! LM's 8x8 editor shows every tile palette-colored and jumps straight from a
//! level's GFX slots into editing them. This window is the slot half of that:
//! it resolves the current level's FG/BG GFX nibble through OBJECTGFXLIST
//! ($00A92B, 26 rows — the `CODE_00AA35` upload loop) and its Sprite GFX
//! nibble through SPRITEGFXLIST ($00A8C3, 26 rows — `UploadSpriteGFX` in
//! `bank_00.asm`), shows each slot's GFX file number and VRAM range, and each
//! row's Edit button opens the 8x8 tile editor on that file with the level's
//! FG (for FG1/FG2/FG3/BG1) or sprite (for SP1-SP4) CGRAM palette row
//! preselected — so the tile grid is colored exactly the way the level shows
//! those tiles.

use egui::Context;
use smwe_rom::objects::{
    object_gfx_list::{OBJECT_SLOT_NAMES, OBJECT_SLOT_VRAM_RANGES},
    sprite_gfx_list::{SPRITE_SLOT_NAMES, SPRITE_SLOT_VRAM_BASES},
};

use super::UiLevelEditor;

impl UiLevelEditor {
    pub(super) fn gfx_slot_browser_window(&mut self, ctx: &Context) {
        if !self.show_gfx_slots {
            return;
        }
        let mut open = self.show_gfx_slots;
        egui::Window::new("Level GFX Slots").open(&mut open).resizable(true).default_size([440.0, 400.0]).show(
            ctx,
            |ui| {
                ui.label("The GFX files the game uploads for this level. Edit jumps into the 8x8 tile editor.");
                ui.separator();

                let fg_tileset = (self.level_properties.fg_bg_gfx as usize).min(25);
                let sp_tileset = (self.level_properties.sprite_gfx as usize).min(25);

                ui.strong(format!("FG/BG GFX — tileset ${fg_tileset:01X} (OBJECTGFXLIST)"));
                let fg_files = self.rom.gfx.object_gfx_list.files_for_object_tileset(fg_tileset);
                let palette_fg = self.level_properties.palette_fg as usize;
                egui::Grid::new("gfx_slots_fg_grid").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (i, &file_num) in fg_files.iter().enumerate() {
                        let (lo, hi) = OBJECT_SLOT_VRAM_RANGES[i];
                        ui.label(OBJECT_SLOT_NAMES[i]);
                        ui.monospace(format!("GFX file {file_num:02X}"));
                        ui.monospace(format!("VRAM {lo:#05X}–{hi:#05X}"));
                        let label = format!("Edit {}", OBJECT_SLOT_NAMES[i]);
                        if ui.small_button(&label).clicked() {
                            self.tile_editor_handoff_note = Some(format!(
                                "From Level GFX Slots: {} (FG/BG tileset ${fg_tileset:01X}) → GFX file {file_num:02X}, palette row {palette_fg}",
                                OBJECT_SLOT_NAMES[i],
                            ));
                            self.open_tile_editor_at(file_num, 0, palette_fg);
                        }
                        ui.end_row();
                    }
                });
                ui.small(format!("Tiles colored with the level's FG palette row ({palette_fg})."));

                ui.separator();

                ui.strong(format!("Sprite GFX — tileset ${sp_tileset:01X} (SPRITEGFXLIST)"));
                let sp_files = self.rom.gfx.sprite_gfx_list.files_for_sprite_tileset(sp_tileset);
                let palette_sprite = self.level_properties.palette_sprite as usize;
                egui::Grid::new("gfx_slots_sp_grid").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (i, &file_num) in sp_files.iter().enumerate() {
                        let base = SPRITE_SLOT_VRAM_BASES[i];
                        ui.label(SPRITE_SLOT_NAMES[i]);
                        ui.monospace(format!("GFX file {file_num:02X}"));
                        ui.monospace(format!("VRAM {base:#05X}–{:#05X}", base + 0x7F));
                        let label = format!("Edit {}", SPRITE_SLOT_NAMES[i]);
                        if ui.small_button(&label).clicked() {
                            self.tile_editor_handoff_note = Some(format!(
                                "From Level GFX Slots: {} (sprite tileset ${sp_tileset:01X}) → GFX file {file_num:02X}, palette row {palette_sprite}",
                                SPRITE_SLOT_NAMES[i],
                            ));
                            self.open_tile_editor_at(file_num, 0, palette_sprite);
                        }
                        ui.end_row();
                    }
                });
                ui.small(format!("Tiles colored with the level's sprite palette row ({palette_sprite})."));
            },
        );
        self.show_gfx_slots = open;
    }
}
