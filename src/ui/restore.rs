//! Restore points + "original ROM" reference copy (Lunar Magic v1.80 parity),
//! plus the LM v3.40 "compress new restore points" and LM v3.70
//! "Do Incremental instead of Full Restores for External Changes" options.
//!
//! LM's Restore menu can optionally track changes to the ROM and let the user
//! revert the ROM to any previous restore point; it needs a reference copy of
//! the original ROM. `RestoreManager` keeps that reference (captured when a
//! ROM is opened) plus any number of named snapshots taken by the user (or
//! automatically before each save when auto-tracking is on).
//!
//! Snapshots are full ROM images in memory (a vanilla SMW ROM is 512 KiB, so
//! even dozens of points are cheap), but each new point is stored the way the
//! current options say: zstd-compressed (LM v3.40, on by default) and/or as a
//! block-level delta against the previous point's image (LM v3.70, on by
//! default). Old raw points keep decoding — [`StoredImage`] carries all three
//! formats, and [`RestoreManager::revert_bytes`] reconstructs transparently.
//!
//! Honest LM v3.70 limit: LM scopes incremental restores to *external* changes
//! against a restore *file*. smw-editor keeps points in memory, has no
//! restore file, and cannot detect external changes, so the delta model
//! applies to new points generally — each incremental point is stored as the
//! 4 KiB blocks that differ from the previous point's full image.
//!
//! Nothing here touches the file system — the main window builds the current
//! image by merging unsaved tab edits and decides what to do with the bytes a
//! point hands back.

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use zstd::bulk;

/// Maximum number of *automatic* (pre-save) restore points kept.
/// Manual points are never pruned.
const MAX_AUTO_POINTS: usize = 20;

/// Block size for incremental (delta) restore points. A point in incremental
/// mode stores only the blocks that differ from the previous point's image.
pub const DELTA_BLOCK_SIZE: usize = 0x1000; // 4 KiB

/// zstd level for point storage (matches `src/undo.rs`).
const ZSTD_LEVEL: i32 = 3;

/// How one restore point's image is held in memory. The variants coexist so
/// points written before compression/incremental storage existed still decode.
pub enum StoredImage {
    /// Full image, uncompressed — points from before the LM v3.40 option
    /// existed, and points taken with compression switched off.
    Raw(Vec<u8>),
    /// Full image, zstd-compressed (LM v3.40 "compress new restore points",
    /// on by default). `full_len` is the uncompressed size.
    Compressed { full_len: usize, data: Vec<u8> },
    /// Incremental point (LM v3.70 "Do Incremental instead of Full Restores
    /// for External Changes", on by default): only the [`DELTA_BLOCK_SIZE`]
    /// blocks that differ from the *previous point's* full image, each
    /// zstd-compressed as `(block_index, compressed_bytes)`.
    Delta { full_len: usize, blocks: Vec<(u32, Vec<u8>)> },
}

impl StoredImage {
    /// Size of the full ROM image this point represents.
    pub fn full_len(&self) -> usize {
        match self {
            StoredImage::Raw(b) => b.len(),
            StoredImage::Compressed { full_len, .. } => *full_len,
            StoredImage::Delta { full_len, .. } => *full_len,
        }
    }

    /// Bytes actually held in memory for this point.
    pub fn stored_len(&self) -> usize {
        match self {
            StoredImage::Raw(b) => b.len(),
            StoredImage::Compressed { data, .. } => data.len(),
            StoredImage::Delta { blocks, .. } => blocks.iter().map(|(_, b)| b.len()).sum(),
        }
    }

    /// Short label for UI rows: what kind of storage backs this point.
    pub fn kind_label(&self) -> &'static str {
        match self {
            StoredImage::Raw(_) => "full",
            StoredImage::Compressed { .. } => "compressed",
            StoredImage::Delta { .. } => "delta",
        }
    }

    /// Decode a self-contained image (raw or compressed full image).
    /// Deltas need their base point and go through
    /// [`RestoreManager::reconstruct_full`] instead.
    fn decode_full(&self) -> Option<Vec<u8>> {
        match self {
            StoredImage::Raw(b) => Some(b.clone()),
            StoredImage::Compressed { full_len, data } => {
                let bytes = bulk::decompress(data, *full_len).ok()?;
                (bytes.len() == *full_len).then_some(bytes)
            }
            StoredImage::Delta { .. } => None,
        }
    }
}

/// Short human-readable byte count, e.g. "148 KiB".
pub fn format_byte_size(n: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;
    if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        // Sub-MiB values read better whole ("148 KiB", not "148.3 KiB").
        format!("{} KiB", n.div_ceil(KIB))
    } else {
        format!("{n} B")
    }
}

/// One named snapshot of the ROM image.
pub struct RestorePoint {
    /// Display name, e.g. "Before boss text edits".
    pub name:      String,
    /// When the snapshot was taken.
    pub created:   SystemTime,
    /// The stored image (raw full, compressed full, or delta).
    pub image:     StoredImage,
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

    /// Size of the full ROM image this point represents.
    pub fn raw_len(&self) -> usize {
        self.image.full_len()
    }

    /// Bytes actually held in memory for this point.
    pub fn stored_len(&self) -> usize {
        self.image.stored_len()
    }

    /// Compact "stored / raw (kind)" summary for UI rows, e.g.
    /// "3 KiB / 512 KiB (delta)".
    pub fn size_summary(&self) -> String {
        format!(
            "{} / {} ({})",
            format_byte_size(self.stored_len()),
            format_byte_size(self.raw_len()),
            self.image.kind_label()
        )
    }
}

pub struct RestoreManager {
    /// Which ROM these points belong to; points are dropped when a different
    /// ROM is opened.
    rom_path:                   Option<PathBuf>,
    /// Reference copy of the ROM file as it was when opened — LM's restore
    /// feature needs this to offer "revert to original".
    original_bytes:             Option<Vec<u8>>,
    points:                     Vec<RestorePoint>,
    /// When true, a restore point of the pre-save image is captured
    /// automatically before every ROM save (LM's "optionally track changes").
    pub auto_track_on_save:     bool,
    /// LM v3.40 "compress new restore points" (Options > Restore Point
    /// Options...). On by default; applies to points created from here on.
    pub compress_new_points:    bool,
    /// LM v3.70 "Do Incremental instead of Full Restores for External
    /// Changes" (Options > Restore Point Options...). On by default; new
    /// points store only changed 4 KiB blocks vs the previous point's image.
    pub incremental_new_points: bool,
}

impl RestoreManager {
    pub fn new() -> Self {
        Self {
            rom_path:               None,
            original_bytes:         None,
            points:                 Vec::new(),
            auto_track_on_save:     false,
            // LM ships both restore options on by default.
            compress_new_points:    true,
            incremental_new_points: true,
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

    /// Total bytes held in memory by all points.
    pub fn total_stored_bytes(&self) -> usize {
        self.points.iter().map(RestorePoint::stored_len).sum()
    }

    /// Total uncompressed image bytes all points represent.
    pub fn total_raw_bytes(&self) -> usize {
        self.points.iter().map(RestorePoint::raw_len).sum()
    }

    /// Snapshot the given full ROM image under `name`.
    pub fn create_point(&mut self, name: String, bytes: Vec<u8>) {
        let image = self.store_new_image(&bytes);
        self.push_point(RestorePoint { name, created: SystemTime::now(), image, automatic: false });
    }

    /// Snapshot the pre-save image automatically (only when auto-tracking is
    /// on). Oldest automatic points are pruned past [`MAX_AUTO_POINTS`].
    pub fn auto_point_before_save(&mut self, bytes: Vec<u8>) {
        if !self.auto_track_on_save {
            return;
        }
        let n = self.points.iter().filter(|p| p.automatic).count() + 1;
        let image = self.store_new_image(&bytes);
        self.push_point(RestorePoint {
            name: format!("Auto — before save #{n}"),
            created: SystemTime::now(),
            image,
            automatic: true,
        });
        self.prune_auto_points();
    }

    /// Store a new image per the current options: a delta against the
    /// previous point's full image when incremental mode is on (falling back
    /// to a full image when there is no previous point or the sizes differ),
    /// otherwise a compressed or raw full image.
    fn store_new_image(&self, bytes: &[u8]) -> StoredImage {
        if self.incremental_new_points && !self.points.is_empty() {
            let prev = self.points.len() - 1;
            if let Some(base) = self.reconstruct_full(prev) {
                if base.len() == bytes.len() {
                    return StoredImage::Delta { full_len: bytes.len(), blocks: delta_blocks(&base, bytes) };
                }
            }
        }
        self.store_full_image(bytes)
    }

    /// Store a full image honoring the compression option (no delta).
    fn store_full_image(&self, bytes: &[u8]) -> StoredImage {
        if self.compress_new_points {
            let data = bulk::compress(bytes, ZSTD_LEVEL).expect("zstd compress of in-memory bytes cannot fail");
            StoredImage::Compressed { full_len: bytes.len(), data }
        } else {
            StoredImage::Raw(bytes.to_vec())
        }
    }

    fn push_point(&mut self, point: RestorePoint) {
        self.points.push(point);
    }

    /// Drop oldest automatic points past [`MAX_AUTO_POINTS`], rebasing any
    /// delta that loses its base point (see [`Self::remove_point_at`]).
    fn prune_auto_points(&mut self) {
        while self.points.iter().filter(|p| p.automatic).count() > MAX_AUTO_POINTS {
            let Some(i) = self.points.iter().position(|p| p.automatic) else { break };
            self.remove_point_at(i);
        }
    }

    /// Reconstruct the full image bytes for point `index`, decompressing and
    /// replaying deltas as needed. Returns `None` if out of range or the
    /// stored data is corrupt.
    fn reconstruct_full(&self, index: usize) -> Option<Vec<u8>> {
        let point = self.points.get(index)?;
        if let Some(full) = point.image.decode_full() {
            return Some(full);
        }
        let StoredImage::Delta { full_len, blocks } = &point.image else { return None };
        let mut base = self.reconstruct_full(index.checked_sub(1)?)?;
        if base.len() != *full_len {
            return None;
        }
        for (block_index, data) in blocks {
            let start = (*block_index as usize) * DELTA_BLOCK_SIZE;
            let block = bulk::decompress(data, DELTA_BLOCK_SIZE).ok()?;
            let end = start + block.len();
            if end > base.len() {
                return None;
            }
            base[start..end].copy_from_slice(&block);
        }
        (base.len() == *full_len).then_some(base)
    }

    /// Bytes to restore for point `index`, or `None` if out of range.
    /// Compressed and delta points are reconstructed transparently.
    pub fn revert_bytes(&self, index: usize) -> Option<Vec<u8>> {
        self.reconstruct_full(index)
    }

    /// Remove point `index` and rebase the following point if it was a delta
    /// against the removed one: a delta's base is the previous point's image,
    /// so the follower is materialized into a full image (honoring the
    /// current compression option) before its base disappears. Returns false
    /// if out of range.
    fn remove_point_at(&mut self, index: usize) -> bool {
        if index >= self.points.len() {
            return false;
        }
        // The follower's full image must be reconstructed while its base
        // still exists.
        let follower_full = self
            .points
            .get(index + 1)
            .filter(|p| matches!(p.image, StoredImage::Delta { .. }))
            .and_then(|_| self.reconstruct_full(index + 1));
        self.points.remove(index);
        let follower_is_delta = self.points.get(index).is_some_and(|p| matches!(p.image, StoredImage::Delta { .. }));
        if follower_is_delta {
            match follower_full {
                Some(full) => {
                    let image = self.store_full_image(&full);
                    self.points[index].image = image;
                }
                // Base unrecoverable (corrupt data): drop the orphaned delta
                // rather than leave a point that can never revert.
                None => {
                    self.points.remove(index);
                }
            }
        }
        true
    }

    /// Delete point `index`; returns false if out of range.
    pub fn delete_point(&mut self, index: usize) -> bool {
        self.remove_point_at(index)
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

/// The [`DELTA_BLOCK_SIZE`] blocks that differ between `base` and `new`
/// (same length), each zstd-compressed.
fn delta_blocks(base: &[u8], new: &[u8]) -> Vec<(u32, Vec<u8>)> {
    debug_assert_eq!(base.len(), new.len());
    base.chunks(DELTA_BLOCK_SIZE)
        .zip(new.chunks(DELTA_BLOCK_SIZE))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (_, b))| {
            let data = bulk::compress(b, ZSTD_LEVEL).expect("zstd compress of in-memory bytes cannot fail");
            (i as u32, data)
        })
        .collect()
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

    /// Deterministic pseudo-random-ish bytes (compressible, like a ROM).
    fn sample_image(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| ((i * 31 + usize::from(seed) * 17) % 251) as u8).collect()
    }

    /// Deterministic xorshift noise (incompressible): delta size assertions
    /// need data where a changed block can't compress away to nothing.
    fn sample_noise(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0xFF) as u8
            })
            .collect()
    }

    #[test]
    fn create_and_revert_round_trip() {
        let mut m = manager_with_rom(&[1, 2, 3]);
        // First point has no base: stored as a compressed full image.
        m.create_point("first".to_string(), vec![4, 5, 6]);
        // Second point is a delta against the first.
        m.create_point("second".to_string(), vec![7, 8, 9]);
        assert_eq!(m.points().len(), 2);
        assert_eq!(m.revert_bytes(0), Some(vec![4, 5, 6]));
        assert_eq!(m.revert_bytes(1), Some(vec![7, 8, 9]));
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

    #[test]
    fn restore_options_default_on() {
        // LM ships both the v3.40 compression option and the v3.70
        // incremental option on by default.
        let m = RestoreManager::new();
        assert!(m.compress_new_points);
        assert!(m.incremental_new_points);
    }

    #[test]
    fn compressed_point_round_trip_and_smaller() {
        let mut m = manager_with_rom(&[0]);
        m.incremental_new_points = false; // isolate compression
        let image = sample_image(0x8000, 1);
        m.create_point("full".to_string(), image.clone());
        let point = &m.points()[0];
        assert!(matches!(point.image, StoredImage::Compressed { .. }));
        assert!(point.stored_len() < point.raw_len(), "compressed must beat raw on ROM-like data");
        assert_eq!(m.revert_bytes(0), Some(image));
    }

    #[test]
    fn raw_point_stored_when_compression_off() {
        let mut m = manager_with_rom(&[0]);
        m.compress_new_points = false;
        m.incremental_new_points = false;
        let image = sample_image(0x1000, 2);
        m.create_point("raw".to_string(), image.clone());
        let point = &m.points()[0];
        assert!(matches!(point.image, StoredImage::Raw(_)));
        assert_eq!(point.stored_len(), point.raw_len());
        assert_eq!(m.revert_bytes(0), Some(image));
    }

    #[test]
    fn delta_point_round_trip_stores_only_changed_blocks() {
        let mut m = manager_with_rom(&[0]);
        let base = sample_noise(0x4000, 3); // 4 blocks
        m.create_point("base".to_string(), base.clone());
        assert!(matches!(m.points()[0].image, StoredImage::Compressed { .. }), "first point is a full image");

        let mut edited = base.clone();
        edited[0x1000] ^= 0xFF; // touch block 1 only
        edited[0x2FFF] ^= 0x0F; // touch block 2 only
        m.create_point("edited".to_string(), edited.clone());

        let StoredImage::Delta { full_len, blocks } = &m.points()[1].image else {
            panic!("second point must be a delta");
        };
        assert_eq!(*full_len, base.len());
        let mut changed: Vec<u32> = blocks.iter().map(|(i, _)| *i).collect();
        changed.sort_unstable();
        assert_eq!(changed, vec![1, 2], "only the two touched blocks are stored");
        assert_eq!(m.revert_bytes(1), Some(edited));
        // A two-block delta is far smaller than a second full image.
        assert!(m.points()[1].stored_len() < m.points()[0].stored_len());
    }

    #[test]
    fn delta_falls_back_to_full_when_sizes_differ() {
        let mut m = manager_with_rom(&[0]);
        m.create_point("small".to_string(), sample_image(0x1000, 4));
        m.create_point("big".to_string(), sample_image(0x2000, 4));
        // Can't delta across sizes: the second point is a full image.
        assert!(matches!(m.points()[1].image, StoredImage::Compressed { .. }));
        assert_eq!(m.revert_bytes(1), Some(sample_image(0x2000, 4)));
    }

    #[test]
    fn deleting_delta_base_rebases_follower() {
        let mut m = manager_with_rom(&[0]);
        let a = sample_noise(0x4000, 5);
        let mut b = a.clone();
        b[0x100] ^= 0xAA;
        m.create_point("a".to_string(), a.clone());
        m.create_point("b".to_string(), b.clone());
        assert!(matches!(m.points()[1].image, StoredImage::Delta { .. }));

        assert!(m.delete_point(0));
        // The orphaned delta was materialized into a full image: revert
        // still returns the exact bytes.
        assert!(!matches!(m.points()[0].image, StoredImage::Delta { .. }));
        assert_eq!(m.revert_bytes(0), Some(b));
    }

    #[test]
    fn pruning_rebases_chained_deltas() {
        let mut m = manager_with_rom(&[0]);
        m.auto_track_on_save = true;
        // Each auto point is a delta against the previous one. (Auto-point
        // names recycle after pruning, so expected images are tracked
        // directly rather than parsed back out of the names.)
        let mut expected_images = Vec::new();
        for i in 0..(MAX_AUTO_POINTS + 3) {
            let mut image = sample_noise(0x2000, 6);
            image[0] = i as u8;
            expected_images.push(image.clone());
            m.auto_point_before_save(image);
        }
        // The 3 oldest points were pruned; every survivor must still revert
        // to its exact bytes.
        assert_eq!(m.points().len(), MAX_AUTO_POINTS);
        for (i, expected) in expected_images[3..].iter().enumerate() {
            assert_eq!(m.revert_bytes(i).as_deref(), Some(expected.as_slice()), "point {i} must survive pruning");
        }
    }

    #[test]
    fn format_byte_size_reads_sensibly() {
        assert_eq!(format_byte_size(0), "0 B");
        assert_eq!(format_byte_size(999), "999 B");
        assert_eq!(format_byte_size(148 * 1024), "148 KiB");
        assert_eq!(format_byte_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
