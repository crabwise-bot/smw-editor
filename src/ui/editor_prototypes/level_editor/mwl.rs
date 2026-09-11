//! Lunar Magic `.mwl` import/export actions for the level editor toolbar.
//!
//! Export renders the current level — including unsaved in-memory edits, by
//! applying them to a scratch copy of the ROM — to an `.mwl` file. Import
//! decodes an `.mwl` file, confirms with the user, writes it into the ROM
//! file (with the same backup + atomic-rename safety as a normal save), and
//! reloads the level from the freshly parsed ROM.

use std::sync::Arc;

use rfd::{MessageButtons, MessageDialog, MessageDialogResult};

use crate::ui::tool::DockableEditorTool;
use smwe_rom::{
    mwl::{self, MwlFile},
    snes_utils::rom::Rom,
    SmwRom,
};

use super::UiLevelEditor;

/// Does this ROM image carry a 0x200-byte SMC copier header?
fn smc_header_offset(rom_bytes: &[u8]) -> usize {
    if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 }
}

/// Write `rom_bytes` back to the ROM file with a `.bak` backup and an atomic
/// temp-file rename, mirroring the main save path.
fn write_rom_file_atomic(rom_path: &std::path::Path, rom_bytes: &[u8]) -> anyhow::Result<()> {
    if rom_path.exists() {
        let bak_path = rom_path.with_extension(format!(
            "{}.bak",
            rom_path.extension().and_then(|e| e.to_str()).unwrap_or("smc")
        ));
        std::fs::copy(rom_path, &bak_path)?;
    }
    let dest_dir = rom_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let tmp_path = dest_dir.join(format!(
        ".{}.tmp",
        rom_path.file_name().and_then(|n| n.to_str()).unwrap_or("rom_save")
    ));
    std::fs::write(&tmp_path, rom_bytes)?;
    std::fs::rename(&tmp_path, rom_path)?;
    Ok(())
}

impl UiLevelEditor {
    /// Export the current level (with unsaved edits applied) to an `.mwl`
    /// file chosen via a save dialog.
    pub(super) fn export_mwl(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Lunar Magic level", &["mwl"])
            .set_file_name(format!("level-{:03X}.mwl", self.level_num))
            .save_file()
        else {
            return;
        };

        // Apply in-memory edits to a scratch copy so the export matches
        // what's on screen, then export from the re-parsed level.
        let result = (|| -> anyhow::Result<String> {
            let mut bytes = self.rom.rom.bytes().to_vec();
            self.save_to_rom(&mut bytes, false)?;
            let scratch = SmwRom::from_rom(Rom::new(bytes)?)?;
            let mwl = mwl::export_level(&scratch, self.level_num as u32)?;
            let encoded = mwl.encode()?;
            std::fs::write(&path, &encoded)?;
            Ok(format!("Exported level {:03X} → {} ({} bytes)", self.level_num, path.display(), encoded.len()))
        })();

        self.mwl_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("MWL export failed: {e:#}"),
        });
    }

    /// Import an `.mwl` file into the current level, with confirmation.
    pub(super) fn import_mwl(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Lunar Magic level", &["mwl"]).pick_file()
        else {
            return;
        };
        let raw = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.mwl_status = Some(format!("MWL import failed: cannot read {}: {e}", path.display()));
                return;
            }
        };
        let mwl = match MwlFile::decode(&raw) {
            Ok(m) => m,
            Err(e) => {
                self.mwl_status = Some(format!("MWL import failed: {e}"));
                return;
            }
        };
        let src_level = match mwl::decode_level_info(&mwl.sections[mwl::SECTION_LEVEL_INFO]) {
            Ok(info) => info.level_num,
            Err(e) => {
                self.mwl_status = Some(format!("MWL import failed: {e}"));
                return;
            }
        };

        let mut prompt = format!(
            "Import '{}' (level {:03X}) into current level {:03X}?\n\nThis overwrites the level's headers, Layer 1, Layer 2 and sprite data in the ROM.",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            src_level,
            self.level_num,
        );
        if self.has_edits {
            prompt.push_str("\n\nYou have unsaved edits that will be discarded.");
        }
        let proceed = MessageDialog::new()
            .set_title("Import .mwl level")
            .set_description(prompt)
            .set_buttons(MessageButtons::OkCancel)
            .show();
        if proceed != MessageDialogResult::Ok {
            return;
        }

        let result = (|| -> anyhow::Result<String> {
            let mut rom_bytes = std::fs::read(&self.rom_path)?;
            let header_offset = smc_header_offset(&rom_bytes);
            mwl::import_level(&mut rom_bytes, &mwl, self.level_num as u32, header_offset)?;
            write_rom_file_atomic(&self.rom_path, &rom_bytes)?;
            // Re-parse so the editor (and its level view) reflect the import.
            let fresh = SmwRom::from_file(&self.rom_path)?;
            self.rom = Arc::new(fresh);
            self.load_level();
            self.has_edits = false;
            Ok(format!(
                "Imported '{}' (level {:03X}) into level {:03X}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                src_level,
                self.level_num
            ))
        })();

        self.mwl_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("MWL import failed: {e:#}"),
        });
    }
}
