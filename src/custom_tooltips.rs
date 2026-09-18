//! Lunar Magic **custom tooltips for objects** (v3.60 parity).
//!
//! LM 3.60 lets the user attach their own description text ("custom
//! tooltips") to objects: hovering an object entry then shows the user's
//! text. The tooltips are editor metadata — they never touch the ROM — so
//! smw-editor keeps them in a per-user JSON store next to the other
//! per-user state (`$HOME/.smw-editor-custom-tooltips.json`, the same
//! convention as `src/project.rs`'s recent-files list).
//!
//! Two independent maps exist because Lunar Magic distinguishes the two
//! object kinds everywhere (the Add Objects window has separate tabs):
//! * standard objects — the 1-byte IDs `0x00`–`0xFF`
//! * extended objects — the `0x00`–`0xFF` IDs placed with the extended format
//!
//! An empty string clears a tooltip; a missing file or malformed JSON
//! loads as "no custom tooltips" (never an error to the UI).

use std::{collections::BTreeMap, path::PathBuf};

/// Which object kind a custom tooltip belongs to. Standard and extended
/// objects share the same `0x00`–`0xFF` ID space, so the kind is part of the
/// key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Standard,
    Extended,
}

impl ObjectKind {
    /// Short label used in the tooltip manager window and in JSON keys.
    pub fn label(self) -> &'static str {
        match self {
            ObjectKind::Standard => "Standard",
            ObjectKind::Extended => "Extended",
        }
    }
}

const STORE_FILENAME: &str = ".smw-editor-custom-tooltips.json";
const MAX_TOOLTIP_LEN: usize = 256;

/// User-settable custom tooltips for level objects (LM v3.60 parity).
#[derive(Debug, Default, Clone)]
pub struct CustomTooltips {
    standard: BTreeMap<u8, String>,
    extended: BTreeMap<u8, String>,
}

impl CustomTooltips {
    /// Load the per-user store; missing or unreadable files (and malformed
    /// JSON) yield an empty store — the UI never fails on this.
    pub fn load() -> Self {
        Self::load_from(&Self::store_path())
    }

    /// Persist the store to the per-user file. Failures are logged, not
    /// propagated — a tooltip edit must never break a save flow.
    pub fn save(&self) {
        if let Err(e) = self.save_to(&Self::store_path()) {
            log::warn!("Failed to save custom tooltips to {}: {e}", Self::store_path().display());
        }
    }

    /// Path of the per-user store.
    pub fn store_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(STORE_FILENAME)
    }

    /// Look up the custom tooltip for an object, if the user set one.
    pub fn get(&self, kind: ObjectKind, id: u8) -> Option<&str> {
        let map = match kind {
            ObjectKind::Standard => &self.standard,
            ObjectKind::Extended => &self.extended,
        };
        map.get(&id).map(String::as_str)
    }

    /// Set (or clear) a tooltip. `text` is trimmed and capped at
    /// [`MAX_TOOLTIP_LEN`] chars; an empty result removes the entry.
    /// Returns `true` when the store changed.
    pub fn set(&mut self, kind: ObjectKind, id: u8, text: &str) -> bool {
        let text: String = text.trim().chars().take(MAX_TOOLTIP_LEN).collect();
        let map = match kind {
            ObjectKind::Standard => &mut self.standard,
            ObjectKind::Extended => &mut self.extended,
        };
        if text.is_empty() {
            return map.remove(&id).is_some();
        }
        if map.get(&id).is_some_and(|v| v == &text) {
            return false; // identical text: no change
        }
        map.insert(id, text);
        true
    }

    /// Total number of custom tooltips stored.
    pub fn len(&self) -> usize {
        self.standard.len() + self.extended.len()
    }

    /// Whether the store holds no custom tooltips.
    pub fn is_empty(&self) -> bool {
        self.standard.is_empty() && self.extended.is_empty()
    }

    /// Iterate `(kind, id, text)` triples in stable ID order (for the
    /// manager window's search listing).
    pub fn iter(&self) -> impl Iterator<Item = (ObjectKind, u8, &str)> {
        let std = self.standard.iter().map(|(&id, t)| (ObjectKind::Standard, id, t.as_str()));
        let ext = self.extended.iter().map(|(&id, t)| (ObjectKind::Extended, id, t.as_str()));
        std.chain(ext)
    }

    fn load_from(path: &std::path::Path) -> Self {
        #[derive(serde::Deserialize, Default)]
        struct StoreFile {
            #[serde(default)]
            standard: BTreeMap<String, String>,
            #[serde(default)]
            extended: BTreeMap<String, String>,
        }
        fn parse_hex_id(s: &str) -> Option<u8> {
            let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
            u8::from_str_radix(s, 16).ok()
        }
        let Ok(data) = std::fs::read_to_string(path) else { return Self::default() };
        let Ok(file) = serde_json::from_str::<StoreFile>(&data) else { return Self::default() };
        let convert = |m: BTreeMap<String, String>| {
            m.into_iter()
                .filter_map(|(k, v)| {
                    let id = parse_hex_id(&k)?;
                    let v: String = v.trim().chars().take(MAX_TOOLTIP_LEN).collect();
                    if v.is_empty() {
                        None
                    } else {
                        Some((id, v))
                    }
                })
                .collect::<BTreeMap<u8, String>>()
        };
        Self { standard: convert(file.standard), extended: convert(file.extended) }
    }

    fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        #[derive(serde::Serialize)]
        struct StoreFile {
            standard: BTreeMap<String, String>,
            extended: BTreeMap<String, String>,
        }
        let hex = |m: &BTreeMap<u8, String>| {
            m.iter().map(|(&id, t)| (format!("{id:02X}"), t.clone())).collect::<BTreeMap<_, _>>()
        };
        let file = StoreFile { standard: hex(&self.standard), extended: hex(&self.extended) };
        let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("smw-editor-tooltips-test-{name}.json"))
    }

    #[test]
    fn set_get_clear_round_trip() {
        let mut tips = CustomTooltips::default();
        assert!(tips.is_empty());
        assert_eq!(tips.get(ObjectKind::Standard, 0x2B), None);

        assert!(tips.set(ObjectKind::Standard, 0x2B, "Question block row"));
        assert!(tips.set(ObjectKind::Extended, 0x0C, "Extended: donut lift"));
        // Same ID in the other kind is independent.
        assert_eq!(tips.get(ObjectKind::Extended, 0x2B), None);
        assert_eq!(tips.get(ObjectKind::Standard, 0x2B), Some("Question block row"));
        assert_eq!(tips.len(), 2);

        // Setting identical text is a no-op.
        assert!(!tips.set(ObjectKind::Standard, 0x2B, "Question block row"));
        // Empty text clears.
        assert!(tips.set(ObjectKind::Standard, 0x2B, "   "));
        assert_eq!(tips.get(ObjectKind::Standard, 0x2B), None);
        assert_eq!(tips.len(), 1);
    }

    #[test]
    fn tooltip_text_is_trimmed_and_capped() {
        let mut tips = CustomTooltips::default();
        let long = "x".repeat(MAX_TOOLTIP_LEN + 50);
        tips.set(ObjectKind::Standard, 0x01, &format!("  {long}  "));
        let got = tips.get(ObjectKind::Standard, 0x01).unwrap();
        assert_eq!(got.chars().count(), MAX_TOOLTIP_LEN);
        assert!(!got.starts_with(' ') && !got.ends_with(' '));
    }

    #[test]
    fn file_round_trip_uses_hex_keys() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let mut tips = CustomTooltips::default();
        tips.set(ObjectKind::Standard, 0x2B, "Question block");
        tips.set(ObjectKind::Extended, 0xFF, "Last extended");
        tips.save_to(&path).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"2B\""));
        assert!(raw.contains("\"FF\""));

        let loaded = CustomTooltips::load_from(&path);
        assert_eq!(loaded.get(ObjectKind::Standard, 0x2B), Some("Question block"));
        assert_eq!(loaded.get(ObjectKind::Extended, 0xFF), Some("Last extended"));
        assert_eq!(loaded.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_or_missing_file_loads_empty() {
        let missing = temp_path("missing-never-created");
        let _ = std::fs::remove_file(&missing);
        assert!(CustomTooltips::load_from(&missing).is_empty());

        let bad = temp_path("malformed");
        std::fs::write(&bad, "{ this is not json").unwrap();
        assert!(CustomTooltips::load_from(&bad).is_empty());

        // Bad IDs and empty values are skipped, good ones survive.
        std::fs::write(&bad, r#"{"standard":{"ZZ":"bad id","2B":"","2C":"ok"},"extended":{}}"#).unwrap();
        let loaded = CustomTooltips::load_from(&bad);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get(ObjectKind::Standard, 0x2C), Some("ok"));
        let _ = std::fs::remove_file(&bad);
    }

    #[test]
    fn iter_yields_stable_order() {
        let mut tips = CustomTooltips::default();
        tips.set(ObjectKind::Extended, 0x03, "e3");
        tips.set(ObjectKind::Standard, 0xFF, "sff");
        tips.set(ObjectKind::Standard, 0x00, "s00");
        let ids: Vec<(ObjectKind, u8)> = tips.iter().map(|(k, id, _)| (k, id)).collect();
        assert_eq!(
            ids,
            vec![(ObjectKind::Standard, 0x00), (ObjectKind::Standard, 0xFF), (ObjectKind::Extended, 0x03),]
        );
    }
}
