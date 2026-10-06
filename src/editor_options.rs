//! Per-user editor options (Lunar Magic v1.91 "Check Object Placement on
//! Save", v3.40 "More ExAnimation Checks", v3.40/v3.70 "Restore Point
//! Options", v3.70 "Allow Descriptive GFX File Names").
//!
//! Stored in `$HOME/.smw-editor-options.json` — the same per-user convention
//! as the recent-files list and the custom-tooltips store. These are editor
//! metadata, never written to the ROM.
//!
//! Missing or unreadable files (and malformed JSON) yield the defaults — the
//! UI never fails on this.

use std::path::PathBuf;

const STORE_FILENAME: &str = ".smw-editor-options.json";

/// Per-user editor options.
#[derive(Debug, Clone, Copy)]
pub struct EditorOptions {
    /// Lunar Magic v1.91 "Check Object Placement on Save" (Options menu).
    /// When on, saving to the ROM warns about objects and sprites placed
    /// outside the level boundaries.
    pub check_placement_on_save:     bool,
    /// Lunar Magic v3.40 "More ExAnimation Checks" (Options menu). When on,
    /// the shared "ExAnimated Frames" dialog warns about ExAnimation
    /// destinations set to disabled slots and duplicate one-shot trigger
    /// numbers. LM ships this checked; unchecking disables the warnings.
    pub more_exanimation_checks:     bool,
    /// Lunar Magic v3.40 "compress new restore points" (Options menu >
    /// "Restore Point Options..."). When on, new restore points are
    /// zstd-compressed in memory. LM ships this checked.
    pub restore_compress_points:     bool,
    /// Lunar Magic v3.70 "Do Incremental instead of Full Restores for
    /// External Changes" (Options menu > "Restore Point Options..."). When
    /// on, a new restore point stores only the 4 KiB blocks that changed vs
    /// the previous point's image. LM ships this checked.
    pub restore_incremental_points:  bool,
    /// Lunar Magic v3.70 "Allow Descriptive GFX File Names" (Options menu).
    /// When on, inserting ExGFX accepts descriptive file names of the form
    /// `ExGFX###T.bin` (e.g. `ExGFX80Mario tiles.bin`) and pre-fills the
    /// file index from the name. LM ships this checked.
    pub allow_descriptive_gfx_names: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        // LM ships every one of these options' defaults this way.
        EditorOptions {
            check_placement_on_save:     false,
            more_exanimation_checks:     true,
            restore_compress_points:     true,
            restore_incremental_points:  true,
            allow_descriptive_gfx_names: true,
        }
    }
}

impl EditorOptions {
    /// Load the per-user store; missing/unreadable/malformed files yield
    /// the defaults.
    pub fn load() -> Self {
        Self::load_from(&Self::store_path())
    }

    /// Persist the options to the per-user file. Failures are logged, not
    /// propagated — an option toggle must never break a save flow.
    pub fn save(&self) {
        if let Err(e) = self.save_to(&Self::store_path()) {
            log::warn!("Failed to save editor options to {}: {e}", Self::store_path().display());
        }
    }

    /// Path of the per-user store.
    pub fn store_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(STORE_FILENAME)
    }

    fn load_from(path: &std::path::Path) -> Self {
        #[derive(serde::Deserialize, Default)]
        struct StoreFile {
            #[serde(default)]
            check_placement_on_save:     bool,
            // `serde(default)` keeps old files (written before this field
            // existed) loading as the default (true).
            #[serde(default = "default_more_exanimation_checks")]
            more_exanimation_checks:     bool,
            // Same story: files written before the restore-point options
            // existed load with both on, matching LM's defaults.
            #[serde(default = "default_restore_compress_points")]
            restore_compress_points:     bool,
            #[serde(default = "default_restore_incremental_points")]
            restore_incremental_points:  bool,
            // Same: files written before the descriptive-GFX-names option
            // existed load with it on (LM v3.70 ships it checked).
            #[serde(default = "default_allow_descriptive_gfx_names")]
            allow_descriptive_gfx_names: bool,
        }
        let Ok(data) = std::fs::read_to_string(path) else { return Self::default() };
        let Ok(file) = serde_json::from_str::<StoreFile>(&data) else { return Self::default() };
        EditorOptions {
            check_placement_on_save:     file.check_placement_on_save,
            more_exanimation_checks:     file.more_exanimation_checks,
            restore_compress_points:     file.restore_compress_points,
            restore_incremental_points:  file.restore_incremental_points,
            allow_descriptive_gfx_names: file.allow_descriptive_gfx_names,
        }
    }

    fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct StoreFile {
            check_placement_on_save:     bool,
            more_exanimation_checks:     bool,
            restore_compress_points:     bool,
            restore_incremental_points:  bool,
            allow_descriptive_gfx_names: bool,
        }
        let file = StoreFile {
            check_placement_on_save:     self.check_placement_on_save,
            more_exanimation_checks:     self.more_exanimation_checks,
            restore_compress_points:     self.restore_compress_points,
            restore_incremental_points:  self.restore_incremental_points,
            allow_descriptive_gfx_names: self.allow_descriptive_gfx_names,
        };
        let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

/// The default for files written before the field existed.
fn default_more_exanimation_checks() -> bool {
    true
}

/// The default for files written before the field existed (LM v3.40 ships it on).
fn default_restore_compress_points() -> bool {
    true
}

/// The default for files written before the field existed (LM v3.70 ships it on).
fn default_restore_incremental_points() -> bool {
    true
}

/// The default for files written before the field existed (LM v3.70 ships it checked).
fn default_allow_descriptive_gfx_names() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("smwe-opt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.json");
        let opts = EditorOptions {
            check_placement_on_save:     true,
            more_exanimation_checks:     false,
            restore_compress_points:     false,
            restore_incremental_points:  false,
            allow_descriptive_gfx_names: false,
        };
        opts.save_to(&path).unwrap();
        let back = EditorOptions::load_from(&path);
        assert!(back.check_placement_on_save);
        assert!(!back.more_exanimation_checks);
        assert!(!back.restore_compress_points);
        assert!(!back.restore_incremental_points);
        assert!(!back.allow_descriptive_gfx_names);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_yields_defaults() {
        let dir = std::env::temp_dir().join(format!("smwe-opt-test-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.json");
        std::fs::write(&path, "{not json").unwrap();
        let back = EditorOptions::load_from(&path);
        assert!(!back.check_placement_on_save);
        assert!(back.more_exanimation_checks);
        assert!(back.restore_compress_points);
        assert!(back.restore_incremental_points);
        assert!(back.allow_descriptive_gfx_names);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn old_file_without_new_field_loads_true() {
        // A file written before `more_exanimation_checks` existed must not
        // silently turn the checks off.
        let dir = std::env::temp_dir().join(format!("smwe-opt-test-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.json");
        std::fs::write(&path, "{\"check_placement_on_save\": true}").unwrap();
        let back = EditorOptions::load_from(&path);
        assert!(back.check_placement_on_save);
        assert!(back.more_exanimation_checks);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn old_file_without_restore_options_loads_both_on() {
        // A file written before the restore-point options existed must load
        // with both on, matching LM's defaults (v3.40/v3.70 ship them on).
        let dir = std::env::temp_dir().join(format!("smwe-opt-test-oldrp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.json");
        std::fs::write(&path, "{\"check_placement_on_save\": false, \"more_exanimation_checks\": true}").unwrap();
        let back = EditorOptions::load_from(&path);
        assert!(!back.check_placement_on_save);
        assert!(back.more_exanimation_checks);
        assert!(back.restore_compress_points);
        assert!(back.restore_incremental_points);
        assert!(back.allow_descriptive_gfx_names);
        std::fs::remove_dir_all(&dir).ok();
    }
}
