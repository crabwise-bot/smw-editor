//! Per-user descriptive names for inserted ExGFX files (Lunar Magic v3.70
//! "Allow Descriptive GFX File Names" parity).
//!
//! LM v3.70 accepts ExGFX file names of the form `ExGFX###T.bin` where `T`
//! is arbitrary descriptive text. smw-editor stores inserted files in ROM
//! free space (the file name is only meaningful during insert), so the
//! descriptive suffix is kept as editor metadata — it never touches the
//! ROM — and shown in the ExGFX Manager's file list next to the file index.
//!
//! The store lives at `$HOME/.smw-editor-exgfx-file-names.json` (the same
//! per-user convention as the custom-tooltips and options stores). A
//! missing file or malformed JSON loads as "no descriptive names" (never
//! an error to the UI); names are keyed by file index (`0x80`-`0xFFF`).

use std::{collections::BTreeMap, path::PathBuf};

const STORE_FILENAME: &str = ".smw-editor-exgfx-file-names.json";

/// User-kept descriptive names for ExGFX files, keyed by file index.
#[derive(Debug, Default, Clone)]
pub struct ExGfxFileNames {
    names: BTreeMap<u16, String>,
}

impl ExGfxFileNames {
    /// Load the per-user store; missing/unreadable/malformed files yield an
    /// empty store — the UI never fails on this.
    pub fn load() -> Self {
        Self::load_from(&Self::store_path())
    }

    /// Persist the store to the per-user file. Failures are logged, not
    /// propagated — a name edit must never break a save flow.
    pub fn save(&self) {
        if let Err(e) = self.save_to(&Self::store_path()) {
            log::warn!("Failed to save ExGFX file names to {}: {e}", Self::store_path().display());
        }
    }

    /// Path of the per-user store.
    pub fn store_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(STORE_FILENAME)
    }

    /// Look up the descriptive name for an ExGFX file index.
    pub fn get(&self, index: u16) -> Option<&str> {
        self.names.get(&index).map(String::as_str)
    }

    /// Set (or clear) a descriptive name. The text is trimmed and capped at
    /// 128 chars; `None`/empty removes the entry.
    pub fn set(&mut self, index: u16, text: Option<&str>) {
        let trimmed = text.map(str::trim).filter(|t| !t.is_empty());
        match trimmed {
            Some(t) => {
                let capped: String = t.chars().take(128).collect();
                self.names.insert(index, capped);
            }
            None => {
                self.names.remove(&index);
            }
        }
    }

    /// Drop the name for a file (e.g. when the file is deleted).
    pub fn remove(&mut self, index: u16) {
        self.names.remove(&index);
    }

    fn load_from(path: &std::path::Path) -> Self {
        #[derive(serde::Deserialize, Default)]
        struct StoreFile {
            #[serde(default)]
            names: BTreeMap<u16, String>,
        }
        let Ok(data) = std::fs::read_to_string(path) else { return Self::default() };
        let Ok(file) = serde_json::from_str::<StoreFile>(&data) else { return Self::default() };
        Self { names: file.names }
    }

    fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct StoreFile<'a> {
            names: &'a BTreeMap<u16, String>,
        }
        let file = StoreFile { names: &self.names };
        let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove() {
        let mut names = ExGfxFileNames::default();
        names.set(0x80, Some("Mario tiles"));
        assert_eq!(names.get(0x80), Some("Mario tiles"));
        names.set(0x80, None);
        assert_eq!(names.get(0x80), None);
        names.set(0x81, Some("  "));
        assert_eq!(names.get(0x81), None);
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("smwe-gfxnames-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("names.json");
        let mut names = ExGfxFileNames::default();
        names.set(0x80, Some("Mario tiles"));
        names.save_to(&path).unwrap();
        let back = ExGfxFileNames::load_from(&path);
        assert_eq!(back.get(0x80), Some("Mario tiles"));
        assert_eq!(back.get(0x81), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_yields_empty() {
        let dir = std::env::temp_dir().join(format!("smwe-gfxnames-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("names.json");
        std::fs::write(&path, "{not json").unwrap();
        let back = ExGfxFileNames::load_from(&path);
        assert_eq!(back.get(0x80), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
