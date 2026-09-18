//! Restore points + "original ROM" reference copy (Lunar Magic v1.80 parity).
//!
//! LM's Restore menu can optionally track changes to the ROM and let the user
//! revert the ROM to any previous restore point; it needs a reference copy of
//! the original ROM. `RestoreManager` keeps that reference (captured when a
//! ROM is opened) plus any number of named full-image snapshots taken by the
//! user (or automatically before each save when auto-tracking is on).
//!
//! Snapshots are full ROM images in memory (a vanilla SMW ROM is 512 KiB, so
//! even dozens of points are cheap). Nothing here touches the file system —
//! the main window builds the current image by merging unsaved tab edits and
//! decides what to do with the bytes a point hands back.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Maximum number of *automatic* (pre-save) restore points kept.
/// Manual points are never pruned.
const MAX_AUTO_POINTS: usize = 20;

/// One named snapshot of the full ROM image.
pub struct RestorePoint {
    /// Display name, e.g. "Before boss text edits".
    pub name:      String,
    /// When the snapshot was taken.
    pub created:   SystemTime,
    /// Full ROM image at snapshot time.
    pub bytes:     Vec<u8>,
    /// True for points created automatically before a save; those are pruned
    /// FIFO past [`MAX_AUTO_POINTS`].
    pub automatic: bool,
}

impl RestorePoint {
    /// Short human-readable age/creation stamp for menu rows.
    pub fn stamp(&self) -> String {
        match self.created.elapsed() {
            Ok(d) => {
                let secs = d.as_secs();
                if secs < 60 {
                    format!("{}s ago", secs.max(1))
                } else if secs < 3600 {
                    format!("{}m ago", secs / 60)
                } else if secs < 86400 {
                    format!("{}h ago", secs / 3600)
                } else {
                    format!("{}d ago", secs / 86400)
                }
            }
            Err(_) => "just now".to_string(),
        }
    }
}

pub struct RestoreManager {
    /// Which ROM these points belong to; points are dropped when a different
    /// ROM is opened.
    rom_path:               Option<PathBuf>,
    /// Reference copy of the ROM file as it was when opened — LM's restore
    /// feature needs this to offer "revert to original".
    original_bytes:         Option<Vec<u8>>,
    points:                 Vec<RestorePoint>,
    /// When true, a restore point of the pre-save image is captured
    /// automatically before every ROM save (LM's "optionally track changes").
    pub auto_track_on_save: bool,
}

impl RestoreManager {
    pub fn new() -> Self {
        Self {
            rom_path:           None,
            original_bytes:     None,
            points:             Vec::new(),
            auto_track_on_save: false,
        }
    }

    /// Called when a ROM is opened: captures the reference copy and drops any
    /// points belonging to a different ROM.
    pub fn open_rom(&mut self, path: &Path) {
        if self.rom_path.as_deref() == Some(path) {
            return;
        }
        self.rom_path = Some(path.to_path_buf());
        self.points.clear();
        self.original_bytes = std::fs::read(path).ok();
    }

    /// Reference copy of the ROM as opened, if the read succeeded.
    pub fn original(&self) -> Option<&[u8]> {
        self.original_bytes.as_deref()
    }

    pub fn points(&self) -> &[RestorePoint] {
        &self.points
    }

    /// Snapshot the given full ROM image under `name`.
    pub fn create_point(&mut self, name: String, bytes: Vec<u8>) {
        self.push_point(RestorePoint { name, created: SystemTime::now(), bytes, automatic: false });
    }

    /// Snapshot the pre-save image automatically (only when auto-tracking is
    /// on). Oldest automatic points are pruned past [`MAX_AUTO_POINTS`].
    pub fn auto_point_before_save(&mut self, bytes: Vec<u8>) {
        if !self.auto_track_on_save {
            return;
        }
        let n = self.points.iter().filter(|p| p.automatic).count() + 1;
        self.push_point(RestorePoint {
            name: format!("Auto — before save #{n}"),
            created: SystemTime::now(),
            bytes,
            automatic: true,
        });
        let auto_count = self.points.iter().filter(|p| p.automatic).count();
        if auto_count > MAX_AUTO_POINTS {
            let drop = auto_count - MAX_AUTO_POINTS;
            let mut dropped = 0;
            self.points.retain(|p| {
                if p.automatic && dropped < drop {
                    dropped += 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    fn push_point(&mut self, point: RestorePoint) {
        self.points.push(point);
    }

    /// Bytes to restore for point `index`, or `None` if out of range.
    pub fn revert_bytes(&self, index: usize) -> Option<&[u8]> {
        self.points.get(index).map(|p| p.bytes.as_slice())
    }

    /// Delete point `index`; returns false if out of range.
    pub fn delete_point(&mut self, index: usize) -> bool {
        if index < self.points.len() {
            self.points.remove(index);
            true
        } else {
            false
        }
    }

    /// Rename point `index`; returns false if out of range or the name is blank.
    pub fn rename_point(&mut self, index: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        match self.points.get_mut(index) {
            Some(p) => {
                p.name = name.to_string();
                true
            }
            None => false,
        }
    }

    /// Default name offered by the "Create Restore Point" dialog.
    pub fn suggested_name(&self) -> String {
        format!("Restore point {}", self.points.len() + 1)
    }
}

impl Default for RestoreManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with_rom(bytes: &[u8]) -> RestoreManager {
        let mut m = RestoreManager::new();
        // Bypass the filesystem: tests can't rely on a ROM file existing.
        m.rom_path = Some(PathBuf::from("/fake/rom.smc"));
        m.original_bytes = Some(bytes.to_vec());
        m
    }

    #[test]
    fn create_and_revert_round_trip() {
        let mut m = manager_with_rom(&[1, 2, 3]);
        m.create_point("first".to_string(), vec![4, 5, 6]);
        m.create_point("second".to_string(), vec![7, 8, 9]);
        assert_eq!(m.points().len(), 2);
        assert_eq!(m.revert_bytes(0), Some([4, 5, 6].as_slice()));
        assert_eq!(m.revert_bytes(1), Some([7, 8, 9].as_slice()));
        assert_eq!(m.revert_bytes(2), None);
    }

    #[test]
    fn rename_and_delete() {
        let mut m = manager_with_rom(&[0]);
        m.create_point("a".to_string(), vec![1]);
        assert!(m.rename_point(0, "  renamed  "));
        assert_eq!(m.points()[0].name, "renamed");
        assert!(!m.rename_point(0, "   "));
        assert!(m.delete_point(0));
        assert!(m.points().is_empty());
        assert!(!m.delete_point(0));
    }

    #[test]
    fn auto_points_pruned_fifo_manual_kept() {
        let mut m = manager_with_rom(&[0]);
        m.auto_track_on_save = true;
        m.create_point("manual".to_string(), vec![9]);
        for _ in 0..(MAX_AUTO_POINTS + 5) {
            m.auto_point_before_save(vec![1]);
        }
        let auto: Vec<_> = m.points().iter().filter(|p| p.automatic).collect();
        assert_eq!(auto.len(), MAX_AUTO_POINTS);
        // The manual point survived pruning.
        assert!(m.points().iter().any(|p| p.name == "manual" && !p.automatic));
        // Oldest auto point was pruned: names count up from 1.
        assert!(!m.points().iter().any(|p| p.name == "Auto — before save #1"));
    }

    #[test]
    fn auto_points_ignored_when_tracking_off() {
        let mut m = manager_with_rom(&[0]);
        m.auto_point_before_save(vec![1]);
        assert!(m.points().is_empty());
    }

    #[test]
    fn opening_different_rom_resets_state() {
        let dir = std::env::temp_dir();
        let a = dir.join("restore_test_a.smc");
        let b = dir.join("restore_test_b.smc");
        std::fs::write(&a, [1, 2, 3]).unwrap();
        std::fs::write(&b, [4, 5, 6]).unwrap();

        let mut m = RestoreManager::new();
        m.open_rom(&a);
        m.create_point("p".to_string(), vec![7]);
        assert_eq!(m.original(), Some([1, 2, 3].as_slice()));

        // Same ROM re-opened: nothing resets.
        m.open_rom(&a);
        assert_eq!(m.points().len(), 1);

        // Different ROM: points cleared, reference re-captured.
        m.open_rom(&b);
        assert!(m.points().is_empty());
        assert_eq!(m.original(), Some([4, 5, 6].as_slice()));

        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }
}
