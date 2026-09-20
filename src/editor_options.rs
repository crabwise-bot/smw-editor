//! Per-user editor options (Lunar Magic v1.91 "Check Object Placement on
//! Save").
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
    pub check_placement_on_save: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        // LM ships the option off by default; the user opts in.
        EditorOptions { check_placement_on_save: false }
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
            check_placement_on_save: bool,
        }
        let Ok(data) = std::fs::read_to_string(path) else { return Self::default() };
        let Ok(file) = serde_json::from_str::<StoreFile>(&data) else { return Self::default() };
        EditorOptions { check_placement_on_save: file.check_placement_on_save }
    }

    fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct StoreFile {
            check_placement_on_save: bool,
        }
        let file = StoreFile { check_placement_on_save: self.check_placement_on_save };
        let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("smwe-opt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("options.json");
        let opts = EditorOptions { check_placement_on_save: true };
        opts.save_to(&path).unwrap();
        let back = EditorOptions::load_from(&path);
        assert!(back.check_placement_on_save);
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
        std::fs::remove_dir_all(&dir).ok();
    }
}
