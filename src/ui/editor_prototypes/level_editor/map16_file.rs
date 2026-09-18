//! Map16 page import/export actions for the Map16 block editor window.
//!
//! Export renders the selected page — including unsaved in-memory block
//! edits, by applying them to a scratch copy of the ROM — to a modern
//! `.map16` file (Lunar Magic 1.90+ `LM16` container; raw 0x800-byte
//! `Map16Page.bin` files remain importable). Import decodes a page file,
//! confirms with the user, writes it into the ROM file (with the same
//! backup + atomic-rename safety as a normal save), and reloads the level
//! from the freshly parsed ROM.
//!
//! Pages: FG 0x00-0x7F / BG 0x00-0x7F. Pages 0x00/0x01 are the vanilla
//! pages; 0x02+ are Lunar Magic 1.70/2.50 expanded pages.

use std::sync::Arc;

use rfd::{MessageButtons, MessageDialog, MessageDialogResult};
use smwe_rom::{
    map16_expanded,
    map16_file::{self, ModernMap16, PageSel},
    objects::tilesets::{object_tileset_to_map16_tileset, TILESETS_COUNT},
    snes_utils::rom::Rom,
    SmwRom,
};

use super::{
    mwl::{smc_header_offset, write_rom_file_atomic},
    UiLevelEditor,
};
use crate::ui::tool::DockableEditorTool;

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

/// Effective act-as table of a scratch ROM: stored table (pending edits are
/// already flushed into the scratch by `save_to_rom`).
fn scratch_acts(scratch: &SmwRom) -> std::collections::HashMap<u16, u16> {
    map16_expanded::read_acts_table(scratch.rom_bytes(), 0).unwrap_or_default()
}

impl UiLevelEditor {
    /// Export the selected page to a modern `.map16` file (LM 1.90+
    /// container holding the page's tiles plus, for FG pages, its act-as
    /// values).
    pub(super) fn export_map16_page(&mut self) {
        let sel = PageSel { fg: self.map16_page_fg, page: self.map16_page };
        let tileset = if sel.fg { self.map16_tileset_idx.min(4) } else { 0 };
        let default_name = format!("map16-{}{:02X}.map16", if sel.fg { "fg" } else { "bg" }, sel.page);
        let Some(path) =
            rfd::FileDialog::new().add_filter("Map16 file", &["map16", "bin"]).set_file_name(default_name).save_file()
        else {
            return;
        };

        let result = (|| -> anyhow::Result<String> {
            let scratch = scratch_rom_with_edits(self)?;
            let raw = map16_file::export_page_sel(&scratch, sel, tileset)?;
            let tiles: Vec<[u8; 8]> = raw.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
            let acts: Vec<u16> = if sel.fg {
                let table = scratch_acts(&scratch);
                (0..256)
                    .map(|i| {
                        let tile = sel.page as u16 * 256 + i as u16;
                        table.get(&tile).copied().unwrap_or(tile)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let data = map16_file::serialize_modern_partial(sel.fg, sel.page, 0x10, &tiles, &acts)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::fs::write(&path, &data)?;
            Ok(format!("Exported {} ({} bytes) → {}", sel.label(), data.len(), path.display()))
        })();

        self.map16_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Map16 export failed: {e:#}"),
        });
    }

    /// Export every Map16 page (FG 0x00-0x7F + BG 0x00-0x7F), the FG act-as
    /// table, and the tileset-specific FG pages 0-1 to one `.map16` file —
    /// Lunar Magic's full-game "export ALL" container.
    pub(super) fn export_map16_all(&mut self) {
        let Some(path) =
            rfd::FileDialog::new().add_filter("Map16 file", &["map16"]).set_file_name("map16-all.map16").save_file()
        else {
            return;
        };

        let result = (|| -> anyhow::Result<String> {
            let scratch = scratch_rom_with_edits(self)?;
            let bytes = scratch.rom_bytes();

            let mut fg_pages = Vec::new();
            for page in 0x02..0x80u8 {
                if let Some(data) =
                    map16_expanded::read_expanded_fg_page(bytes, 0, page).map_err(|e| anyhow::anyhow!("{e}"))?
                {
                    if data.iter().any(|&b| b != 0) {
                        fg_pages.push((page, data.to_vec()));
                    }
                }
            }
            let mut bg_pages = Vec::new();
            for page in 0x00..0x80u8 {
                let data = map16_file::export_page_sel(&scratch, PageSel { fg: false, page }, 0)?;
                if data.iter().any(|&b| b != 0) {
                    bg_pages.push((page, data));
                }
            }
            let mut ts_group_pages = [[0u8; 0x1000]; TILESETS_COUNT];
            for (ts, slot) in ts_group_pages.iter_mut().enumerate() {
                let p0 = map16_file::export_page(&scratch, map16_file::PAGE_FG0, ts)?;
                let p1 = map16_file::export_page(&scratch, map16_file::PAGE_FG1, ts)?;
                slot[..0x800].copy_from_slice(&p0);
                slot[0x800..].copy_from_slice(&p1);
            }
            let acts = scratch_acts(&scratch);
            let input = map16_file::FullExportInput {
                fg_pages:       &fg_pages,
                bg_pages:       &bg_pages,
                acts:           &acts,
                ts_group_pages: &ts_group_pages,
            };
            let data = map16_file::serialize_modern_full(&input);
            std::fs::write(&path, &data)?;
            Ok(format!(
                "Exported ALL Map16 ({} FG pages, {} BG pages, {} act-as entries) → {} ({} bytes)",
                fg_pages.len(),
                bg_pages.len(),
                acts.len(),
                path.display(),
                data.len()
            ))
        })();

        self.map16_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Map16 export-all failed: {e:#}"),
        });
    }

    /// Import Map16 files: raw 0x800-byte pages, modern partial `.map16`
    /// files (single page or tile rectangle), and full-game "export ALL"
    /// files. Multiple files can be picked at once.
    pub(super) fn import_map16(&mut self) {
        let paths = rfd::FileDialog::new().add_filter("Map16 file", &["map16", "bin"]).pick_files();
        let Some(paths) = paths else { return };
        if paths.is_empty() {
            return;
        }
        for path in paths {
            let raw = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    self.map16_file_status = Some(format!("Map16 import failed: cannot read {}: {e}", path.display()));
                    return;
                }
            };
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let sel = PageSel { fg: self.map16_page_fg, page: self.map16_page };
            let tileset = self.map16_tileset_idx.min(4);
            let outcome: anyhow::Result<String> = (|| {
                match map16_file::detect_map16_file_kind(&raw) {
                    map16_file::Map16FileKind::RawPage => {
                        // Raw pages carry no location: they go to the selected page.
                        if !self.confirm_import(&format!(
                            "Import '{file_name}' (raw 0x800-byte page) into {} (tileset {tileset})?",
                            sel.label()
                        )) {
                            return Ok(format!("Skipped '{file_name}'"));
                        }
                        self.write_import(|rom_bytes, header_offset| {
                            map16_file::import_page_sel(rom_bytes, sel, tileset, &raw, header_offset)
                        })?;
                        Ok(format!("Imported '{file_name}' → {}", sel.label()))
                    }
                    map16_file::Map16FileKind::Modern => {
                        let m = map16_file::parse_modern_map16(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
                        if m.flags & map16_file::MH_FULL_EXPORT != 0 {
                            self.import_modern_full(&file_name, &m)
                        } else if let Some(loc) = map16_file::modern_partial_page(&m) {
                            self.import_modern_partial(&file_name, &m, loc, sel, tileset)
                        } else {
                            self.import_modern_rect(&file_name, &m)
                        }
                    }
                    map16_file::Map16FileKind::Unknown => {
                        anyhow::bail!("'{file_name}': not a Map16 file (want LM16 container or 0x800 raw bytes)")
                    }
                }
            })();
            match outcome {
                Ok(msg) => {
                    log::info!("{msg}");
                    self.map16_file_status = Some(msg);
                }
                Err(e) => {
                    self.map16_file_status = Some(format!("Map16 import failed: {e:#}"));
                    return;
                }
            }
        }
    }

    /// Ok/Cancel confirmation for a destructive import.
    fn confirm_import(&self, prompt: &str) -> bool {
        let mut full = prompt.to_string();
        if self.has_edits {
            full.push_str("\n\nYou have unsaved edits that will be discarded.");
        }
        MessageDialog::new()
            .set_title("Import Map16")
            .set_description(full)
            .set_buttons(MessageButtons::OkCancel)
            .show()
            == MessageDialogResult::Ok
    }

    /// Run `write` against the ROM file (with backup + atomic rename),
    /// then reload the editor from the freshly parsed ROM.
    fn write_import(
        &mut self, write: impl FnOnce(&mut [u8], usize) -> Result<(), map16_file::Map16FileError>,
    ) -> anyhow::Result<()> {
        let mut rom_bytes = std::fs::read(&self.rom_path)?;
        let header_offset = smc_header_offset(&rom_bytes);
        write(&mut rom_bytes, header_offset).map_err(|e| anyhow::anyhow!("{e}"))?;
        write_rom_file_atomic(&self.rom_path, &rom_bytes)?;
        let fresh = SmwRom::from_file(&self.rom_path)?;
        self.rom = Arc::new(fresh);
        self.map16_edits = crate::undo::UndoableData::new(super::map16_editor::EditableMap16Edits::default());
        self.map16_acts_edits.clear();
        self.load_level();
        self.has_edits = false;
        Ok(())
    }

    /// Import one modern partial-export page. Defaults to the file's own
    /// page; offers the currently selected page as an alternative.
    fn import_modern_partial(
        &mut self, file_name: &str, m: &ModernMap16, loc: map16_file::ModernPartialPage, sel: PageSel, tileset: usize,
    ) -> anyhow::Result<String> {
        let file_sel = PageSel { fg: loc.fg, page: loc.page };
        let dest = if file_sel == sel {
            file_sel
        } else {
            let choice = MessageDialog::new()
                .set_title("Import Map16 page")
                .set_description(format!(
                    "'{file_name}' is an export of {}.\n\nYes = import to the file's own page.\nNo = import to the selected page ({}).",
                    file_sel.label(),
                    sel.label()
                ))
                .set_buttons(MessageButtons::YesNoCancel)
                .show();
            match choice {
                MessageDialogResult::Yes => file_sel,
                MessageDialogResult::No => sel,
                _ => return Ok(format!("Skipped '{file_name}'")),
            }
        };
        let tiles: Vec<[u8; 8]> = m.tile_data.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
        if tiles.is_empty() {
            anyhow::bail!("'{file_name}': partial export has no tile data");
        }
        // Tileset only matters for vanilla FG pages 0x00/0x01.
        let use_tileset = if dest.fg && dest.page < 0x02 { tileset } else { 0 };
        let raw: Vec<u8> = tiles.iter().flat_map(|t| t.iter().copied()).collect();
        self.write_import(|rom_bytes, header_offset| {
            map16_file::import_page_sel(rom_bytes, dest, use_tileset, &raw, header_offset)?;
            if dest.fg && !m.act_data.is_empty() {
                let mut table = map16_expanded::read_acts_table(rom_bytes, header_offset)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?;
                for (i, chunk) in m.act_data.chunks_exact(2).enumerate() {
                    let tile = dest.page as u16 * 256 + i as u16;
                    let act = u16::from_le_bytes([chunk[0], chunk[1]]);
                    map16_expanded::set_act_as(&mut table, tile, (act != tile).then_some(act));
                }
                map16_expanded::write_acts_table(rom_bytes, header_offset, &table)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?;
            }
            Ok(())
        })?;
        Ok(format!("Imported '{file_name}' → {}", dest.label()))
    }

    /// Import a modern partial export that is a sub-page tile rectangle
    /// (variable tile counts). Pastes at the file's own base coordinates.
    fn import_modern_rect(&mut self, file_name: &str, m: &ModernMap16) -> anyhow::Result<String> {
        let (fg, base_y) = if m.flags & map16_file::MH_FG_RELATIVE != 0 {
            (true, m.base_y)
        } else if m.flags & map16_file::MH_BG_RELATIVE != 0 {
            (false, m.base_y)
        } else if (0x400..0x800).contains(&m.base_y) {
            (false, m.base_y - 0x400)
        } else {
            (true, m.base_y)
        };
        let (sx, sy) = (m.size_x, m.size_y);
        if sx == 0 || sx > 0x10 || sy == 0 || m.base_x + sx > 0x10 {
            anyhow::bail!("'{file_name}': unsupported selection ({sx}×{sy} at {},{})", m.base_x, m.base_y);
        }
        let page = (base_y >> 4) as u8;
        if page >= 0x80 || (base_y + sy - 1) >> 4 != base_y >> 4 {
            anyhow::bail!("'{file_name}': selection spans pages (only single-page selections import)");
        }
        let tiles: Vec<[u8; 8]> = m.tile_data.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
        if tiles.len() as u32 != sx * sy {
            anyhow::bail!("'{file_name}': tile data ({} tiles) doesn't match {sx}×{sy}", tiles.len());
        }
        let dest = PageSel { fg, page };
        if !self.confirm_import(&format!(
            "Import '{file_name}' ({sx}×{sy} tiles at {} tile ({},{}))?",
            dest.label(),
            m.base_x,
            base_y & 0xF
        )) {
            return Ok(format!("Skipped '{file_name}'"));
        }
        self.write_import(|rom_bytes, header_offset| {
            if fg {
                let mut page_bytes = map16_expanded::read_expanded_fg_page(rom_bytes, header_offset, page)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?
                    .unwrap_or([0u8; map16_file::MAP16_PAGE_BYTES]);
                for (i, tile) in tiles.iter().enumerate() {
                    let dx = i as u32 % sx;
                    let dy = i as u32 / sx;
                    let idx = ((base_y & 0xF) + dy) * 0x10 + m.base_x + dx;
                    let off = idx as usize * 8;
                    page_bytes[off..off + 8].copy_from_slice(tile);
                }
                map16_expanded::write_expanded_fg_page(rom_bytes, header_offset, page, &page_bytes)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?;
            } else {
                let mut page_bytes = map16_expanded::read_expanded_bg_page(rom_bytes, header_offset, page)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?
                    .unwrap_or([0u8; map16_file::MAP16_PAGE_BYTES]);
                for (i, tile) in tiles.iter().enumerate() {
                    let dx = i as u32 % sx;
                    let dy = i as u32 / sx;
                    let idx = ((base_y & 0xF) + dy) * 0x10 + m.base_x + dx;
                    let off = idx as usize * 8;
                    page_bytes[off..off + 8].copy_from_slice(tile);
                }
                map16_expanded::write_expanded_bg_page(rom_bytes, header_offset, page, &page_bytes)
                    .map_err(|e| map16_file::Map16FileError::Modern(e.to_string()))?;
            }
            Ok(())
        })?;
        Ok(format!("Imported '{file_name}' → {} tile ({},{})", dest.label(), m.base_x, base_y & 0xF))
    }

    /// Import a full-game "export ALL" file: every FG/BG page, the FG act-as
    /// table, and the tileset-specific FG pages 0-1.
    fn import_modern_full(&mut self, file_name: &str, m: &ModernMap16) -> anyhow::Result<String> {
        if m.tile_data.len() != map16_file::FULL_EXPORT_TILE_BYTES {
            anyhow::bail!("'{file_name}': full export has wrong tile blob size {:#X}", m.tile_data.len());
        }
        let mut fg_pages = 0;
        let mut bg_pages = 0;
        for page in 0x02..0x80u8 {
            let off = page as usize * map16_file::MAP16_PAGE_BYTES;
            if m.tile_data[off..off + map16_file::MAP16_PAGE_BYTES].iter().any(|&b| b != 0) {
                fg_pages += 1;
            }
        }
        for page in 0x00..0x80u8 {
            let off = 0x40000 + page as usize * map16_file::MAP16_PAGE_BYTES;
            if m.tile_data[off..off + map16_file::MAP16_PAGE_BYTES].iter().any(|&b| b != 0) {
                bg_pages += 1;
            }
        }
        if !self.confirm_import(&format!(
            "Import ALL Map16 from '{file_name}'?\n\n{fg_pages} FG pages, {bg_pages} BG pages, act-as table, tileset pages 0-1.\nThis overwrites all Map16 data in the ROM."
        )) {
            return Ok(format!("Skipped '{file_name}'"));
        }
        self.write_import(|rom_bytes, header_offset| {
            let cvt = |e: map16_expanded::ExpandedMap16Error| map16_file::Map16FileError::Modern(e.to_string());
            // FG expanded pages 0x02-0x7F.
            for page in 0x02..0x80u8 {
                let off = page as usize * map16_file::MAP16_PAGE_BYTES;
                let slice = &m.tile_data[off..off + map16_file::MAP16_PAGE_BYTES];
                if slice.iter().any(|&b| b != 0) {
                    map16_expanded::write_expanded_fg_page(rom_bytes, header_offset, page, slice).map_err(cvt)?;
                }
            }
            // BG pages (vanilla 0x00/0x01 + expanded 0x02-0x7F).
            for page in 0x00..0x80u8 {
                let off = 0x40000 + page as usize * map16_file::MAP16_PAGE_BYTES;
                let slice = &m.tile_data[off..off + map16_file::MAP16_PAGE_BYTES];
                if slice.iter().any(|&b| b != 0) {
                    if page < 0x02 {
                        map16_file::import_page(rom_bytes, map16_file::PAGE_BG0 + page, 0, slice, header_offset)?;
                    } else {
                        map16_expanded::write_expanded_bg_page(rom_bytes, header_offset, page, slice).map_err(cvt)?;
                    }
                }
            }
            // FG act-as table (full replace: the export stores identity for
            // untouched tiles).
            if !m.act_data.is_empty() {
                let mut table = std::collections::HashMap::new();
                for (i, chunk) in m.act_data.chunks_exact(2).enumerate() {
                    let tile = i as u16;
                    if tile >= 0x8000 {
                        break;
                    }
                    let act = u16::from_le_bytes([chunk[0], chunk[1]]);
                    map16_expanded::set_act_as(&mut table, tile, (act != tile).then_some(act));
                }
                map16_expanded::write_acts_table(rom_bytes, header_offset, &table).map_err(cvt)?;
            }
            // Tileset-group-specific FG pages 0-1 (groups 0-4 are real).
            if !m.ts_group_data.is_empty() {
                for ts in 0..TILESETS_COUNT {
                    let off = ts * 0x1000;
                    if off + 0x1000 > m.ts_group_data.len() {
                        break;
                    }
                    let group = &m.ts_group_data[off..off + 0x1000];
                    if group.iter().any(|&b| b != 0) {
                        map16_file::import_page(rom_bytes, map16_file::PAGE_FG0, ts, &group[..0x800], header_offset)?;
                        map16_file::import_page(rom_bytes, map16_file::PAGE_FG1, ts, &group[0x800..], header_offset)?;
                    }
                }
            }
            Ok(())
        })?;
        Ok(format!("Imported ALL Map16 from '{file_name}' ({fg_pages} FG pages, {bg_pages} BG pages)"))
    }

    /// Render the import/export controls at the bottom of the Map16 editor
    /// window. Call inside the window's `show` closure.
    pub(super) fn map16_file_controls(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Page import/export");
        ui.label("Modern .map16 files are Lunar Magic 1.90+ compatible; raw 0x800-byte pages still import.");

        ui.horizontal(|ui| {
            ui.label("Page:");
            ui.radio_value(&mut self.map16_page_fg, true, "FG");
            ui.radio_value(&mut self.map16_page_fg, false, "BG");
            let mut page = self.map16_page.min(0x7F) as i32;
            if ui.add(egui::Slider::new(&mut page, 0..=0x7F).hexadecimal(2, false, true)).changed() {
                self.map16_page = page as u8;
            }
            let kind = if self.map16_page < 0x02 { "vanilla" } else { "expanded" };
            ui.small(format!("({kind})"));
        });

        if self.map16_page_fg && self.map16_page < 0x02 {
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
            if ui.button("Export ALL…").clicked() {
                self.export_map16_all();
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
