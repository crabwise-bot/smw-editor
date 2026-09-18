//! "Change Properties in Sprite Header" dialog (Lunar Magic parity).
//!
//! LM's sprite-header dialog edits the 1-byte sprite header that precedes
//! each level's sprite data (bit layout verified against the vanilla game —
//! see [`smwe_rom::level::headers::SpriteHeader`]):
//!
//! * sprite memory setting (bits 0-5; vanilla levels use 0x00-0x12, the
//!   19-entry `SpriteSlotMax/Start` tables in `bank_02.asm`),
//! * disable Layer 2 interaction (bit 6),
//! * sprite buoyancy (bit 7).
//!
//! LM v3.00 added two more controls — sprite vertical spawning range and
//! smart spawning — implemented as an LM-inserted ASM patch. smw-editor does
//! not install that patch, so those two are stored in the editor-native
//! [`smwe_rom::level::sprite_header_ext`] RATS block as authoring intent
//! (same honest boundary as the LM v3.00 level-height slice, PR #34).
//!
//! The dialog also shows the level's sprite count against LM 3.00's 128
//! sprite cap for non-SA1 ROMs (up from 84).

use egui::{ComboBox, Context};
use smwe_rom::level::sprite_header_ext::{SpawnRange, MAX_SPRITES_LM300};

use super::UiLevelEditor;

/// Sprite memory values the vanilla game can index (`SpriteSlotMax/Start`
/// tables in `bank_02.asm` have 19 entries, 0x00-0x12). LM's own dropdown
/// stops at 0x09, but vanilla levels use up to 0x12, so the editor offers
/// the full valid range rather than clobbering those levels on save.
const SPRITE_MEMORY_MAX: u8 = 0x12;

impl UiLevelEditor {
    pub(super) fn sprite_header_editor_window(&mut self, ctx: &Context) {
        if !self.show_sprite_header_editor {
            return;
        }
        let mut open = self.show_sprite_header_editor;
        egui::Window::new("Change Properties in Sprite Header").open(&mut open).resizable(false).show(ctx, |ui| {
            let sprite_count = self.sprites.read(|s| s.sprites.len());
            ui.label(format!(
                "Level {:03X} — {sprite_count} sprite{} (LM 3.00 limit: {MAX_SPRITES_LM300} for non-SA1 ROMs)",
                self.level_num,
                if sprite_count == 1 { "" } else { "s" },
            ));
            if sprite_count > MAX_SPRITES_LM300 {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!(
                        "Warning: {sprite_count} sprites exceeds LM 3.00's {MAX_SPRITES_LM300}-sprite cap; \
                             LM would refuse to save this level."
                    ),
                );
            }
            ui.separator();

            let mut changed = false;

            // ── Sprite memory setting ──
            let mut mem = self.sprite_header_edit.sprite_memory();
            let mem_before = mem;
            ComboBox::from_label("Sprite memory setting").selected_text(format!("0x{mem:02X}")).show_ui(ui, |ui| {
                for v in 0..=SPRITE_MEMORY_MAX {
                    let label = if v <= 0x09 {
                        format!("0x{v:02X}")
                    } else {
                        format!("0x{v:02X} (extended, used by vanilla levels)")
                    };
                    ui.selectable_value(&mut mem, v, label);
                }
            });
            if mem != mem_before {
                self.sprite_header_edit.set_sprite_memory(mem);
                changed = true;
            }
            ui.small("Which sprite-slot allocation table the level uses (game reads bits 0-5).");

            // ── Bit 6 / bit 7 ──
            let mut l2 = self.sprite_header_edit.disable_layer2_interaction();
            if ui.checkbox(&mut l2, "Disable Layer 2 interaction").changed() {
                self.sprite_header_edit.set_disable_layer2_interaction(l2);
                changed = true;
            }
            let mut buoy = self.sprite_header_edit.sprite_buoyancy();
            if ui
                .checkbox(&mut buoy, "Sprite buoyancy")
                .on_hover_text("Sprites float in water (also affects some platform sprites)")
                .changed()
            {
                self.sprite_header_edit.set_sprite_buoyancy(buoy);
                changed = true;
            }

            ui.separator();
            ui.strong("Lunar Magic 3.00 options");

            // ── Vertical spawning range / smart spawning ──
            // Session-authoritative working copy: only this dialog writes
            // these, and `self.rom` is not refreshed after a save.
            let level_num = self.level_num;
            let mut ext = self.sprite_header_ext.get(level_num);
            let range_before = ext.spawn_range;
            ComboBox::from_label("Sprite vertical spawning range").selected_text(ext.spawn_range.label()).show_ui(
                ui,
                |ui| {
                    for r in SpawnRange::ALL {
                        ui.selectable_value(&mut ext.spawn_range, r, r.label());
                    }
                },
            );
            if ext.spawn_range != range_before {
                changed = true;
            }

            if ui
                .checkbox(&mut ext.smart_spawning, "Smart spawning")
                .on_hover_text("LM 3.00's improved spawn logic (matters most in levels taller than vanilla)")
                .changed()
            {
                changed = true;
            }
            ui.small(
                "These two need Lunar Magic 3.00+'s sprite engine to take effect in-game; \
                     on a stock ROM they are stored as authoring intent. The header-byte fields \
                     above work on any ROM.",
            );

            ui.separator();
            ui.horizontal(|ui| {
                ui.small(format!("Raw header byte: 0x{:02X}", self.sprite_header_edit.as_byte()));
                if changed {
                    ui.small("(unsaved)");
                }
            });

            if changed {
                // `set` clears the entry when both options are default.
                if let Err(e) = self.sprite_header_ext.set(level_num, ext) {
                    log::warn!("Sprite header options: {e}");
                }
                // Record the header byte as session-authoritative (see
                // `sprite_header_edits`).
                self.sprite_header_edits.insert(level_num, self.sprite_header_edit.clone());
                self.sprite_header_dirty = true;
                self.has_edits = true;
            }
        });
        self.show_sprite_header_editor = open;
    }
}
