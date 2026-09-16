//! Map16 page import/export actions for the Map16 block editor window.
//!
//! Export renders the selected page — including unsaved in-memory block
//! edits, by applying them to a scratch copy of the ROM — to a raw
//! 0x800-byte page file (Lunar Magic `Map16Page.bin` compatible). Import
//! decodes a raw page file, confirms with the user, writes it into the ROM
//! file at the page's fixed vanilla address (with the same backup +
//! atomic-rename safety as a normal save), and reloads the level from the
//! freshly parsed ROM.

use std::sync::Arc;

use rfd::{MessageButtons, MessageDialog, MessageDialogResult};
use smwe_rom::{map16_file, objects::tilesets::object_tileset_to_map16_tileset, snes_utils::rom::Rom, SmwRom};

use super::{
    mwl::{smc_header_offset, write_rom_file_atomic},
    UiLevelEditor,
};
use crate::ui::tool::DockableEditorTool;

/// Pages offered by the Map16 editor's page selector.
pub(super) const PAGE_OPTIONS: [(u8, &str); 4] = [
    (map16_file::PAGE_FG0, "FG page 0 (tiles 000-0FF)"),
    (map16_file::PAGE_FG1, "FG page 1 (tiles 100-1FF)"),
    (map16_file::PAGE_BG0, "BG page 0 (table tiles 00-FF)"),
    (map16_file::PAGE_BG1, "BG page 1 (table tiles 100-1FF)"),
];

pub(super) const TILESET_NAMES: [&str; 5] =
    ["0: Normal", "1: Castle", "2: Rope", "3: Underground", "4: Switch Palace/Ghost House"];

/// The Map16 tileset variant the current level uses.
fn current_level_tileset(editor: &UiLevelEditor) -> usize {
    object_tileset_to_map16_tileset(editor.level_properties.fg_bg_gfx as usize)
}

/// Apply in-memory edits to a scratch copy and re-parse, so exports match
/// what's on screen.
fn scratch_rom_with_edits(editor: &UiLevelEditor) -> anyhow::Result<SmwRom> {
    let mut bytes = editor.rom.rom.bytes().to_vec();
    editor.save_to_rom(&mut bytes, false)?;
    Ok(SmwRom::from_rom(Rom::new(bytes)?)?)
}

impl UiLevelEditor {
    /// Export the selected page to a raw 0x800-byte file (LM-compatible).
    pub(super) fn export_map16_page(&mut self) {
        let (page, _) = PAGE_OPTIONS[self.map16_page_idx.min(PAGE_OPTIONS.len() - 1)];
        let tileset = if map16_file::page_is_foreground(page) { self.map16_tileset_idx.min(4) } else { 0 };
        let default_name = if map16_file::page_is_foreground(page) {
            format!("map16-page{:02X}-ts{tileset}.bin", page)
        } else {
            format!("map16-page{:02X}.bin", page)
        };
        let Some(path) =
            rfd::FileDialog::new().add_filter("Map16 page", &["bin"]).set_file_name(default_name).save_file()
        else {
            return;
        };

        let result = (|| -> anyhow::Result<String> {
            let scratch = scratch_rom_with_edits(self)?;
            let data = map16_file::export_page(&scratch, page, tileset)?;
            std::fs::write(&path, &data)?;
            Ok(format!(
                "Exported {} (tileset {tileset}) → {} ({} bytes)",
                map16_file::page_name(page),
                path.display(),
                data.len()
            ))
        })();

        self.map16_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Map16 export failed: {e:#}"),
        });
    }

    /// Import a raw 0x800-byte page file into the ROM, with confirmation.
    /// The file goes into the currently selected page/tileset.
    pub(super) fn import_map16(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Map16 page", &["bin"]).pick_file() else {
            return;
        };
        let raw = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.map16_file_status = Some(format!("Map16 import failed: cannot read {}: {e}", path.display()));
                return;
            }
        };
        if raw.len() != map16_file::MAP16_PAGE_BYTES {
            self.map16_file_status =
                Some(format!("Map16 import failed: expected a 0x800-byte page file, got {} bytes", raw.len()));
            return;
        }

        let (page, _) = PAGE_OPTIONS[self.map16_page_idx.min(PAGE_OPTIONS.len() - 1)];
        let tileset = self.map16_tileset_idx.min(4);
        let describe = format!("{} (tileset {tileset})", map16_file::page_name(page));
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let mut prompt = format!(
            "Import '{file_name}' ({describe}) into the ROM?\n\nThis overwrites Map16 data at fixed ROM addresses."
        );
        if self.has_edits {
            prompt.push_str("\n\nYou have unsaved edits that will be discarded.");
        }
        let proceed = MessageDialog::new()
            .set_title("Import Map16 page")
            .set_description(prompt)
            .set_buttons(MessageButtons::OkCancel)
            .show();
        if proceed != MessageDialogResult::Ok {
            return;
        }

        let result = (|| -> anyhow::Result<String> {
            let mut rom_bytes = std::fs::read(&self.rom_path)?;
            let header_offset = smc_header_offset(&rom_bytes);
            map16_file::import_page(&mut rom_bytes, page, tileset, &raw, header_offset)?;
            write_rom_file_atomic(&self.rom_path, &rom_bytes)?;
            // Re-parse so the editor reflects the import; drop stale edits.
            let fresh = SmwRom::from_file(&self.rom_path)?;
            self.rom = Arc::new(fresh);
            self.map16_edits.clear();
            self.load_level();
            self.has_edits = false;
            Ok(format!("Imported '{file_name}' ({describe})"))
        })();

        self.map16_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Map16 import failed: {e:#}"),
        });
    }

    /// Render the import/export controls at the bottom of the Map16 editor
    /// window. Call inside the window's `show` closure.
    pub(super) fn map16_file_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Page import/export");
        ui.label("Raw 0x800-byte page files are Lunar Magic Map16Page.bin compatible.");

        ui.horizontal(|ui| {
            ui.label("Page:");
            let mut idx = self.map16_page_idx.min(PAGE_OPTIONS.len() - 1);
            egui::ComboBox::from_id_salt("map16_page_sel").selected_text(PAGE_OPTIONS[idx].1).show_ui(ui, |ui| {
                for (i, (_, name)) in PAGE_OPTIONS.iter().enumerate() {
                    ui.selectable_value(&mut idx, i, *name);
                }
            });
            self.map16_page_idx = idx;
        });

        let (page, _) = PAGE_OPTIONS[self.map16_page_idx.min(PAGE_OPTIONS.len() - 1)];
        if map16_file::page_is_foreground(page) {
            ui.horizontal(|ui| {
                ui.label("Tileset:");
                let mut ts = self.map16_tileset_idx.min(4);
                egui::ComboBox::from_id_salt("map16_tileset_sel").selected_text(TILESET_NAMES[ts]).show_ui(ui, |ui| {
                    for (i, name) in TILESET_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut ts, i, *name);
                    }
                });
                self.map16_tileset_idx = ts;
                let cur = current_level_tileset(self);
                ui.small(format!("(this level uses tileset {cur})"));
            });
        }

        ui.horizontal(|ui| {
            if ui.button("Export page…").clicked() {
                self.export_map16_page();
            }
            if ui.button("Import…").clicked() {
                self.import_map16();
            }
        });

        if let Some(status) = self.map16_file_status.clone() {
            ui.label(egui::RichText::new(status).monospace().color(egui::Color32::LIGHT_GREEN));
        }
    }
}
