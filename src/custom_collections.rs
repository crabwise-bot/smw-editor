//! User-defined **"Custom Collections of Objects"** (Lunar Magic v3.60).
//!
//! LM 3.60 re-added the "Custom Collections of Objects" category to the
//! "Add Objects" window (it briefly appeared in LM 1.00, 25 years earlier):
//! a per-user list of named custom extended objects, handy for storing the
//! 3-byte extended-object definitions that control various level settings.
//!
//! This module is the pure, UI-free data model both the level editor's
//! manager window and the headless screenshot binary share, so the two can
//! never drift. The store is editor configuration, not ROM data: it lives in
//! the platform config dir as JSON (LM keeps it in the Windows registry).
//!
//! A *collection* is a named group; each *entry* is a named extended-object
//! ID byte. Placing an entry inserts a 3-byte extended object
//! (`N00YYYYY 0000XXXX BBBBBBBB`) at the clicked tile, like LM's Add Objects
//! window does. Bytes `0x00`/`0x01` are refused: on reload the object stream
//! would decode them as an exit / screen jump, corrupting the level.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One named custom extended object: the third byte of the 3-byte extended
/// object definition (the X/Y come from placement).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustomObjectEntry {
    pub name:        String,
    /// Extended object ID byte (`BBBBBBBB` in the 3-byte entry).
    pub extended_id: u8,
}

/// A named group of custom extended objects.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustomCollection {
    pub name:    String,
    pub entries: Vec<CustomObjectEntry>,
}

/// The whole per-user store.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CustomCollections {
    pub collections: Vec<CustomCollection>,
}

/// Extended-object ID bytes that would decode as something else on reload
/// and must never be stored: `0x00` = exit, `0x01` = screen jump
/// (see `smwe_rom::objects::Object::is_exit` / `is_screen_jump`).
pub const FORBIDDEN_IDS: [u8; 2] = [0x00, 0x01];

impl CustomCollections {
    /// Platform config dir for the store, without new dependencies:
    /// `%APPDATA%\smwe` on Windows, `~/Library/Application Support/smwe` on
    /// macOS, `$XDG_CONFIG_HOME/smwe` (else `~/.config/smwe`) elsewhere.
    pub fn default_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            return base.join("smwe").join("custom_collections.json");
        }
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            return home.join("Library").join("Application Support").join("smwe").join("custom_collections.json");
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
                home.join(".config")
            });
            return base.join("smwe").join("custom_collections.json");
        }
    }

    /// Load from the default path; a missing or corrupt file yields an empty
    /// store (never an error — a bad config must not brick the editor).
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Load from an explicit path (used by tests and the screenshot binary).
    pub fn load_from(path: &std::path::Path) -> Self {
        let Ok(data) = std::fs::read(path) else { return Self::default() };
        serde_json::from_slice(&data).unwrap_or_default()
    }

    /// Save to the default path, creating parent dirs as needed.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Save to an explicit path (used by tests and the screenshot binary).
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, data)
    }

    /// Add a collection; returns its index. Empty names are refused.
    pub fn add_collection(&mut self, name: &str) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Collection name cannot be empty".to_string());
        }
        self.collections.push(CustomCollection { name: name.to_string(), entries: Vec::new() });
        Ok(self.collections.len() - 1)
    }

    /// Rename a collection; returns false when the index is out of range.
    pub fn rename_collection(&mut self, index: usize, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Collection name cannot be empty".to_string());
        }
        let Some(c) = self.collections.get_mut(index) else {
            return Err("No such collection".to_string());
        };
        c.name = name.to_string();
        Ok(())
    }

    /// Delete a collection; returns false when the index is out of range.
    pub fn remove_collection(&mut self, index: usize) -> bool {
        if index >= self.collections.len() {
            return false;
        }
        self.collections.remove(index);
        true
    }

    /// Add an entry to a collection. IDs `0x00`/`0x01` are refused (they
    /// would decode as exit / screen jump on reload).
    pub fn add_entry(&mut self, collection: usize, name: &str, extended_id: u8) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Entry name cannot be empty".to_string());
        }
        if FORBIDDEN_IDS.contains(&extended_id) {
            return Err(format!(
                "ID {extended_id:#04X} is reserved (exit / screen jump) and cannot be a custom object"
            ));
        }
        let Some(c) = self.collections.get_mut(collection) else {
            return Err("No such collection".to_string());
        };
        c.entries.push(CustomObjectEntry { name: name.to_string(), extended_id });
        Ok(())
    }

    /// Replace an entry's name/ID; same validation as [`Self::add_entry`].
    pub fn update_entry(&mut self, collection: usize, entry: usize, name: &str, extended_id: u8) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Entry name cannot be empty".to_string());
        }
        if FORBIDDEN_IDS.contains(&extended_id) {
            return Err(format!(
                "ID {extended_id:#04X} is reserved (exit / screen jump) and cannot be a custom object"
            ));
        }
        let Some(e) = self.collections.get_mut(collection).and_then(|c| c.entries.get_mut(entry)) else {
            return Err("No such entry".to_string());
        };
        e.name = name.to_string();
        e.extended_id = extended_id;
        Ok(())
    }

    /// Delete an entry; returns false when out of range.
    pub fn remove_entry(&mut self, collection: usize, entry: usize) -> bool {
        let Some(c) = self.collections.get_mut(collection) else {
            return false;
        };
        if entry >= c.entries.len() {
            return false;
        }
        c.entries.remove(entry);
        true
    }
}

/// Parse a user-typed extended-object ID as hex: `E0`, `$E0`, or `0xE0`.
/// Anything else (or out of `u8` range) is an error message.
pub fn parse_extended_id(text: &str) -> Result<u8, String> {
    let t = text.trim().trim_start_matches('$');
    let digits = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u8::from_str_radix(digits, 16).map_err(|_| format!("\"{}\" is not a valid hex byte (00–FF)", text.trim()))
}

/// Format an extended-object ID the way the UI shows it.
pub fn format_extended_id(id: u8) -> String {
    format!("{id:#04X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CustomCollections {
        let mut s = CustomCollections::default();
        s.add_collection("Level settings").unwrap();
        s.add_entry(0, "Fast scroll command", 0xE0).unwrap();
        s.add_entry(0, "Darken screen command", 0xE5).unwrap();
        s.add_collection("Boss tricks").unwrap();
        s.add_entry(1, "Custom boss HP", 0xF2).unwrap();
        s
    }

    #[test]
    fn json_round_trip() {
        let s = sample();
        let json = serde_json::to_string_pretty(&s).unwrap();
        let back: CustomCollections = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("smwe-cc-test-{}", std::process::id()));
        let path = dir.join("custom_collections.json");
        let s = sample();
        s.save_to(&path).unwrap();
        let back = CustomCollections::load_from(&path);
        assert_eq!(back, s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_is_empty() {
        let back = CustomCollections::load_from(std::path::Path::new("/nonexistent/smwe-cc.json"));
        assert_eq!(back, CustomCollections::default());
    }

    #[test]
    fn load_corrupt_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("smwe-cc-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom_collections.json");
        std::fs::write(&path, b"not json {{{").unwrap();
        let back = CustomCollections::load_from(&path);
        assert_eq!(back, CustomCollections::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_collection_rejects_empty_name() {
        let mut s = CustomCollections::default();
        assert!(s.add_collection("   ").is_err());
        assert!(s.add_collection("Ok").is_ok());
        assert_eq!(s.collections.len(), 1);
    }

    #[test]
    fn rename_and_remove_collection() {
        let mut s = sample();
        s.rename_collection(0, "Renamed").unwrap();
        assert_eq!(s.collections[0].name, "Renamed");
        assert!(s.rename_collection(0, "").is_err());
        assert!(s.rename_collection(99, "x").is_err());
        assert!(s.remove_collection(1));
        assert_eq!(s.collections.len(), 1);
        assert!(!s.remove_collection(5));
    }

    #[test]
    fn entry_ids_00_and_01_are_refused() {
        let mut s = sample();
        // 0x00 would decode as an exit, 0x01 as a screen jump on reload.
        assert!(s.add_entry(0, "exit?", 0x00).is_err());
        assert!(s.add_entry(0, "jump?", 0x01).is_err());
        assert!(s.update_entry(0, 0, "exit?", 0x00).is_err());
        // Everything else is fine.
        assert!(s.add_entry(0, "fine", 0x02).is_ok());
        assert!(s.add_entry(0, "fine", 0xFF).is_ok());
    }

    #[test]
    fn add_entry_rejects_empty_name_and_bad_collection() {
        let mut s = sample();
        assert!(s.add_entry(0, "  ", 0xE0).is_err());
        assert!(s.add_entry(99, "x", 0xE0).is_err());
    }

    #[test]
    fn update_and_remove_entry() {
        let mut s = sample();
        s.update_entry(0, 0, "New name", 0xD0).unwrap();
        assert_eq!(s.collections[0].entries[0].name, "New name");
        assert_eq!(s.collections[0].entries[0].extended_id, 0xD0);
        assert!(s.update_entry(0, 0, "", 0xD0).is_err());
        assert!(s.update_entry(0, 99, "x", 0xD0).is_err());
        assert!(s.remove_entry(0, 0));
        assert_eq!(s.collections[0].entries.len(), 1);
        assert!(!s.remove_entry(0, 99));
        assert!(!s.remove_entry(99, 0));
    }

    #[test]
    fn parse_extended_id_accepts_hex_forms() {
        assert_eq!(parse_extended_id("E0").unwrap(), 0xE0);
        assert_eq!(parse_extended_id("$e0").unwrap(), 0xE0);
        assert_eq!(parse_extended_id("0xE0").unwrap(), 0xE0);
        assert_eq!(parse_extended_id("  ff  ").unwrap(), 0xFF);
        assert_eq!(parse_extended_id("00").unwrap(), 0x00);
        assert!(parse_extended_id("").is_err());
        assert!(parse_extended_id("xyz").is_err());
        assert!(parse_extended_id("100").is_err()); // > 0xFF
        assert!(parse_extended_id("-1").is_err());
    }

    #[test]
    fn default_path_ends_with_expected_filename() {
        let p = CustomCollections::default_path();
        assert_eq!(p.file_name().unwrap(), "custom_collections.json");
    }
}
