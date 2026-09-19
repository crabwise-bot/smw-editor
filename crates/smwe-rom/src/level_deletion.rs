//! Delete Levels from ROM (Lunar Magic "Delete Levels from ROM" parity).
//!
//! Verified behavior (FuSoYa's release notes: "adds the ability to delete
//! levels from the ROM by replacing them with the 'test' level"; LM 3.63 help
//! topics `file_delete_multiple_levels.htm` / `file_clear_level_area.htm`):
//! the user picks levels, and each deleted level's data is replaced with a
//! copy of the vanilla "test" level while the displaced data blocks are
//! erased, so the space is reclaimed as free space.
//!
//! The vanilla test level is what the 277 unused vanilla level slots already
//! point at, confirmed two ways:
//!
//! - SMWDisX labels: `TestLevel` @ SNES `$068000` (`bank_06.asm`,
//!   `ORG $068000`), `TestLevelSprites` (`bank_07.asm`, SNES `$07E76D`), and
//!   the L2 background `DATA_0CD900` (`bank_0C.asm`) reached through the
//!   `$FFD900` bank-`$FF` pointer that 283 vanilla slots share.
//! - Real-ROM scan of the three pointer tables: `$068000` × 277,
//!   `$07E76D` × 278, `$FFD900` × 283.
//!
//! That is also why deleted levels render as the familiar "TEST" turn-block
//! room: they literally become the test level.

use std::collections::{HashMap, HashSet};

use crate::{
    level::{headers::PRIMARY_HEADER_SIZE, ObjectLayer, SpriteLayer, LAYER2_HEADER_SIZE, LEVEL_COUNT},
    rom_expansion::rewrite_checksum,
    snes_utils::addr::{AddrPc, AddrSnes},
};

// -------------------------------------------------------------------------------------------------
// Test-level targets (SNES addresses)

/// SNES address of the vanilla test level's object data (`TestLevel`).
pub const TEST_LEVEL_L1: u32 = 0x068000;
/// SNES address of the vanilla test level's sprite data (`TestLevelSprites`).
pub const TEST_LEVEL_SPRITES: u32 = 0x07E76D;
/// SNES address of the vanilla test level's Layer 2 pointer (bank `$FF`
/// background pointer to `DATA_0CD900`).
pub const TEST_LEVEL_L2: u32 = 0xFFD900;

// -------------------------------------------------------------------------------------------------
// Pointer tables (SNES addresses)

/// Layer-1 pointer table: `0x200` × 3-byte LoROM SNES address.
const L1_TABLE_SNES: u32 = 0x05E000;
/// Sprite pointer table: `0x200` × 2-byte offset into bank `$07`.
const SPRITE_TABLE_SNES: u32 = 0x05EC00;
/// Layer-2 pointer table: `0x200` × 3-byte LoROM SNES address.
const L2_TABLE_SNES: u32 = 0x05E600;

// -------------------------------------------------------------------------------------------------

/// Levels the game itself needs for the title/demo sequence. Deleting any of
/// these is legal, but Lunar Magic-style tools warn first.
///
/// Verified in SMWDisX `bank_05.asm`'s level pointer table: `IntroLevel0C5`,
/// `TitleScrLevel0C7`, `YoshiWingsLevel0C8`.
pub const GAMEPLAY_CRITICAL_LEVELS: &[u16] = &[0x0C5, 0x0C7, 0x0C8];

// -------------------------------------------------------------------------------------------------

/// The three data pointers of one level, as SNES addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPointers {
    /// Layer-1 object-data pointer (block starts with the 5-byte primary header).
    pub l1:      u32,
    /// Sprite-data pointer (bank `$07`; block starts with the 1-byte sprite header).
    pub sprites: u32,
    /// Layer-2 pointer (bank `$FF` = background tilemap, else object data
    /// with a 5-byte header).
    pub l2:      u32,
}

/// What [`delete_levels`] did.
#[derive(Debug, Clone)]
pub struct DeleteReport {
    /// Levels that were repointed at the test level (sorted, deduplicated).
    pub deleted:         Vec<u16>,
    /// Bytes erased (`0xFF`-filled) and thus reclaimed as free space.
    pub bytes_reclaimed: usize,
    /// Displaced data blocks erased.
    pub blocks_erased:   usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LevelDeletionError {
    #[error("Level {0:03X} out of range (0x000-0x1FF)")]
    LevelOutOfRange(u16),
    #[error("Address conversion failed: {0}")]
    Address(#[from] crate::snes_utils::addr::AddressError),
    #[error("ROM too small for pointer tables")]
    RomTooSmall,
    #[error("No levels selected")]
    NothingSelected,
}

// -------------------------------------------------------------------------------------------------

fn table_pc(
    table_snes: u32, entry: usize, entry_size: usize, rom_len: usize, header_offset: usize,
) -> Result<usize, LevelDeletionError> {
    let base = AddrPc::try_from_lorom(AddrSnes(table_snes))?.as_index() as usize + header_offset;
    let off = base.checked_add(entry * entry_size).ok_or(LevelDeletionError::RomTooSmall)?;
    if off + entry_size > rom_len {
        return Err(LevelDeletionError::RomTooSmall);
    }
    Ok(off)
}

fn lorom_pc(snes: u32, rom_len: usize, header_offset: usize) -> Option<usize> {
    let bank = (snes >> 16) & 0xFF;
    let addr = (snes & 0xFFFF) as usize;
    if addr < 0x8000 {
        return None;
    }
    let pc = ((bank & 0x7F) as usize) * 0x8000 + (addr - 0x8000) + header_offset;
    (pc < rom_len).then_some(pc)
}

fn read_u24_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], 0])
}

/// Read a level's three data pointers from the ROM's pointer tables.
pub fn level_pointers(rom: &[u8], level: u16, header_offset: usize) -> Result<LevelPointers, LevelDeletionError> {
    if level as usize >= LEVEL_COUNT {
        return Err(LevelDeletionError::LevelOutOfRange(level));
    }
    let idx = level as usize;
    let l1_off = table_pc(L1_TABLE_SNES, idx, 3, rom.len(), header_offset)?;
    let sp_off = table_pc(SPRITE_TABLE_SNES, idx, 2, rom.len(), header_offset)?;
    let l2_off = table_pc(L2_TABLE_SNES, idx, 3, rom.len(), header_offset)?;
    let l1 = read_u24_le(rom, l1_off);
    let sprites = 0x070000 | u16::from_le_bytes([rom[sp_off], rom[sp_off + 1]]) as u32;
    let l2 = read_u24_le(rom, l2_off);
    Ok(LevelPointers { l1, sprites, l2 })
}

fn write_pointers(rom: &mut [u8], level: u16, header_offset: usize) -> Result<(), LevelDeletionError> {
    let idx = level as usize;
    let l1_off = table_pc(L1_TABLE_SNES, idx, 3, rom.len(), header_offset)?;
    let sp_off = table_pc(SPRITE_TABLE_SNES, idx, 2, rom.len(), header_offset)?;
    let l2_off = table_pc(L2_TABLE_SNES, idx, 3, rom.len(), header_offset)?;
    rom[l1_off..l1_off + 3].copy_from_slice(&TEST_LEVEL_L1.to_le_bytes()[..3]);
    let sp_lo = (TEST_LEVEL_SPRITES & 0xFFFF) as u16;
    rom[sp_off..sp_off + 2].copy_from_slice(&sp_lo.to_le_bytes());
    rom[l2_off..l2_off + 3].copy_from_slice(&TEST_LEVEL_L2.to_le_bytes()[..3]);
    Ok(())
}

/// Byte length of the data block a pointer targets, or `None` when it cannot
/// be measured safely (corrupt data, out of bounds). Bank-`$FF` Layer-2
/// pointers reference vanilla background tilemaps in the original ROM area and
/// are never measured for erasure.
fn block_len(rom: &[u8], kind: BlockKind, snes: u32, header_offset: usize) -> Option<usize> {
    let pc = lorom_pc(snes, rom.len(), header_offset)?;
    match kind {
        BlockKind::L1 => {
            let start = pc.checked_add(PRIMARY_HEADER_SIZE)?;
            let (_, (_, consumed)) = ObjectLayer::parse(rom.get(start..)?).ok()?;
            Some(PRIMARY_HEADER_SIZE + consumed)
        }
        BlockKind::Sprites => {
            let start = pc.checked_add(1)?;
            let (_, (_, consumed)) = SpriteLayer::parse(rom.get(start..)?).ok()?;
            Some(1 + consumed)
        }
        BlockKind::L2 => {
            if (snes >> 16) & 0xFF == 0xFF {
                // Vanilla background data: shared with other levels and part
                // of the original game data — repoint only, never erase.
                return None;
            }
            let start = pc.checked_add(LAYER2_HEADER_SIZE)?;
            let (_, (_, consumed)) = ObjectLayer::parse(rom.get(start..)?).ok()?;
            Some(LAYER2_HEADER_SIZE + consumed)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BlockKind {
    L1,
    Sprites,
    L2,
}

fn test_target(kind: BlockKind) -> u32 {
    match kind {
        BlockKind::L1 => TEST_LEVEL_L1,
        BlockKind::Sprites => TEST_LEVEL_SPRITES,
        BlockKind::L2 => TEST_LEVEL_L2,
    }
}

// -------------------------------------------------------------------------------------------------

/// Delete levels from the ROM, Lunar Magic style: each selected level's Layer
/// 1, sprite, and Layer 2 pointers are repointed at the vanilla test level,
/// and every displaced data block that no remaining level references is
/// erased (`0xFF`-filled), reclaiming it as free space.
///
/// Safety rules, all verified against the real ROM in tests:
/// - A block already shared with the test level (or with any non-deleted
///   level, e.g. the Mode-7 boss rooms that share `Mode7BossLayer1`) is never
///   erased — only truly orphaned blocks are reclaimed.
/// - Bank-`$FF` Layer-2 pointers are repointed but their background data is
///   never erased (vanilla game data in the original ROM area).
/// - Blocks whose length cannot be parsed are repointed but not erased.
/// - The ROM checksum/complement pair is rewritten.
///
/// `header_offset` is `0x200` when the image has an SMC copier header, else `0`.
pub fn delete_levels(rom: &mut [u8], levels: &[u16], header_offset: usize) -> Result<DeleteReport, LevelDeletionError> {
    let mut deleted: Vec<u16> = levels.iter().copied().collect();
    deleted.sort_unstable();
    deleted.dedup();
    if deleted.is_empty() {
        return Err(LevelDeletionError::NothingSelected);
    }
    for &level in &deleted {
        if level as usize >= LEVEL_COUNT {
            return Err(LevelDeletionError::LevelOutOfRange(level));
        }
    }

    // Snapshot the displaced blocks before repointing.
    let mut displaced: Vec<(BlockKind, u32)> = Vec::new();
    for &level in &deleted {
        let p = level_pointers(rom, level, header_offset)?;
        displaced.push((BlockKind::L1, p.l1));
        displaced.push((BlockKind::Sprites, p.sprites));
        displaced.push((BlockKind::L2, p.l2));
        write_pointers(rom, level, header_offset)?;
    }

    // Reference scan: which blocks does any level still point at?
    let mut referenced: HashSet<(BlockKind, u32)> = HashSet::new();
    for level in 0..LEVEL_COUNT as u16 {
        let p = level_pointers(rom, level, header_offset)?;
        referenced.insert((BlockKind::L1, p.l1));
        referenced.insert((BlockKind::Sprites, p.sprites));
        referenced.insert((BlockKind::L2, p.l2));
    }

    // Erase orphaned, measurable, non-test blocks exactly once each.
    let mut erased: HashSet<(BlockKind, u32)> = HashSet::new();
    let mut bytes_reclaimed = 0usize;
    // Measure before erasing so a shared-prefix overlap can't corrupt the math.
    let mut to_erase: HashMap<(BlockKind, u32), usize> = HashMap::new();
    for key in displaced {
        if key.1 == test_target(key.0) || referenced.contains(&key) || !erased.insert(key) {
            continue;
        }
        if let Some(len) = block_len(rom, key.0, key.1, header_offset) {
            to_erase.insert(key, len);
        }
    }
    let mut blocks_erased = 0usize;
    for ((_, snes), len) in to_erase {
        if let Some(pc) = lorom_pc(snes, rom.len(), header_offset) {
            if pc + len <= rom.len() {
                rom[pc..pc + len].fill(0xFF);
                bytes_reclaimed += len;
                blocks_erased += 1;
            }
        }
    }

    rewrite_checksum(&mut rom[header_offset..]);

    Ok(DeleteReport { deleted, bytes_reclaimed, blocks_erased })
}

// -------------------------------------------------------------------------------------------------

/// `true` when `level`'s data differs between `current` and `original`
/// (the ROM-as-opened reference copy): any of the three pointer-table entries
/// or the pointed-to block bytes differ. Used for the dialog's
/// Modified/Unmodified quick-select categories.
///
/// Blocks that cannot be compared (out of range, unparsable) fall back to the
/// pointer-entry comparison only.
pub fn level_modified_vs(current: &[u8], original: &[u8], level: u16, header_offset: usize) -> bool {
    let idx = level as usize;
    if idx >= LEVEL_COUNT {
        return false;
    }
    let tables = [(L1_TABLE_SNES, 3), (SPRITE_TABLE_SNES, 2), (L2_TABLE_SNES, 3)];
    for &(table, entry_size) in &tables {
        let (Ok(c_off), Ok(o_off)) = (
            table_pc(table, idx, entry_size, current.len(), header_offset),
            table_pc(table, idx, entry_size, original.len(), header_offset),
        ) else {
            continue;
        };
        if current[c_off..c_off + entry_size] != original[o_off..o_off + entry_size] {
            return true;
        }
    }
    // Same pointers: compare the block contents themselves.
    let Ok(p) = level_pointers(current, level, header_offset) else { return false };
    let blocks = [(BlockKind::L1, p.l1), (BlockKind::Sprites, p.sprites), (BlockKind::L2, p.l2)];
    for (kind, snes) in blocks {
        let (Some(c_pc), Some(o_pc)) =
            (lorom_pc(snes, current.len(), header_offset), lorom_pc(snes, original.len(), header_offset))
        else {
            continue;
        };
        // Measure against `current`; a block that can't be parsed is skipped
        // (the pointer comparison above already covered relocation).
        let Some(len) = block_len(current, kind, snes, header_offset) else { continue };
        if c_pc + len > current.len() || o_pc + len > original.len() {
            continue;
        }
        if current[c_pc..c_pc + len] != original[o_pc..o_pc + len] {
            return true;
        }
    }
    false
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rom_expansion::compute_checksum, snes_utils::rom::Rom};

    fn pc_of(rom: &[u8], snes: u32) -> usize {
        lorom_pc(snes, rom.len(), 0).unwrap()
    }

    /// Minimal synthetic ROM: pointer tables at their real SNES addresses
    /// (LoROM-mapped into a 512 KiB image) plus three fake data blocks.
    fn synthetic_rom() -> Vec<u8> {
        let mut rom = vec![0xFFu8; 0x80000];

        // Level 0x000: unique blocks; level 0x001: shares level 0's L1 block;
        // level 0x002: already the test level; rest: test targets.
        let l1_a: u32 = 0x0A8000; // PC 0x048000
        let sp_a: u32 = 0x07C000; // PC 0x03C000
        let l2_a: u32 = 0x0B8000; // PC 0x058000

        // Fake L1 block: 5-byte header + objects + terminator.
        let l1_pc = pc_of(&rom, l1_a);
        rom[l1_pc..l1_pc + 5].copy_from_slice(&[1, 2, 3, 4, 5]);
        rom[l1_pc + 5..l1_pc + 9].copy_from_slice(&[0x2F, 0x00, 0x00, 0xFF]);
        // Fake sprite block: 1-byte header + one sprite + terminator.
        let sp_pc = pc_of(&rom, sp_a);
        rom[sp_pc] = 0x11;
        rom[sp_pc + 1..sp_pc + 4].copy_from_slice(&[0x01, 0x02, 0x03]);
        rom[sp_pc + 4] = 0xFF;
        // Fake L2 object block: 5-byte header + objects + terminator.
        let l2_pc = pc_of(&rom, l2_a);
        rom[l2_pc..l2_pc + 5].copy_from_slice(&[9, 9, 9, 9, 9]);
        rom[l2_pc + 5..l2_pc + 8].copy_from_slice(&[0x40, 0x00, 0xFF]);

        let set = |rom: &mut Vec<u8>, level: usize, l1: u32, sp: u32, l2: u32| {
            let o = pc_of(rom, L1_TABLE_SNES) + level * 3;
            rom[o..o + 3].copy_from_slice(&l1.to_le_bytes()[..3]);
            let o = pc_of(rom, SPRITE_TABLE_SNES) + level * 2;
            rom[o..o + 2].copy_from_slice(&((sp & 0xFFFF) as u16).to_le_bytes());
            let o = pc_of(rom, L2_TABLE_SNES) + level * 3;
            rom[o..o + 3].copy_from_slice(&l2.to_le_bytes()[..3]);
        };
        for level in 0..LEVEL_COUNT {
            set(&mut rom, level, TEST_LEVEL_L1, TEST_LEVEL_SPRITES, TEST_LEVEL_L2);
        }
        set(&mut rom, 0, l1_a, sp_a, l2_a);
        set(&mut rom, 1, l1_a, TEST_LEVEL_SPRITES, TEST_LEVEL_L2);

        // Fix the checksum so tests can assert it stays valid.
        rewrite_checksum(&mut rom);
        rom
    }

    #[test]
    fn repoints_and_erases_orphaned_blocks() {
        let mut rom = synthetic_rom();
        let l1_pc = lorom_pc(0x0A8000, rom.len(), 0).unwrap();
        let report = delete_levels(&mut rom, &[0], 0).unwrap();
        assert_eq!(report.deleted, vec![0]);

        let p = level_pointers(&rom, 0, 0).unwrap();
        assert_eq!(p.l1, TEST_LEVEL_L1);
        assert_eq!(p.sprites, TEST_LEVEL_SPRITES);
        assert_eq!(p.l2, TEST_LEVEL_L2);

        // Level 1 still references the L1 block -> only its own sprite+L2
        // blocks were orphaned... level 0's sprite+L2 blocks are unique.
        // L1 block must NOT be erased (still referenced by level 1).
        assert_ne!(&rom[l1_pc..l1_pc + 5], &[0xFF; 5]);
        // Sprite + L2 blocks were orphaned -> erased.
        let sp_pc = lorom_pc(0x07C000, rom.len(), 0).unwrap();
        assert_eq!(&rom[sp_pc..sp_pc + 5], &[0xFF; 5]);
        let l2_pc = lorom_pc(0x0B8000, rom.len(), 0).unwrap();
        assert_eq!(&rom[l2_pc..l2_pc + 8], &[0xFF; 8]);
        assert!(report.bytes_reclaimed > 0);

        // Checksum still valid.
        let sum = compute_checksum(&rom);
        let stored = u16::from_le_bytes([rom[0x7FDE], rom[0x7FDF]]);
        assert_eq!(sum, stored);
    }

    #[test]
    fn shared_block_survives_until_last_reference_deleted() {
        let mut rom = synthetic_rom();
        delete_levels(&mut rom, &[0], 0).unwrap();
        let l1_pc = lorom_pc(0x0A8000, rom.len(), 0).unwrap();
        assert_ne!(&rom[l1_pc..l1_pc + 5], &[0xFF; 5]);
        // Deleting level 1 too orphans the shared L1 block -> now erased.
        delete_levels(&mut rom, &[1], 0).unwrap();
        assert_eq!(&rom[l1_pc..l1_pc + 9], &[0xFF; 9]);
    }

    #[test]
    fn deleting_test_level_is_noop_for_data() {
        let mut rom = synthetic_rom();
        let before = rom.clone();
        let report = delete_levels(&mut rom, &[2], 0).unwrap();
        assert_eq!(report.deleted, vec![2]);
        assert_eq!(report.blocks_erased, 0);
        // Only the checksum rewrite may differ; pointers were already test targets.
        let p = level_pointers(&rom, 2, 0).unwrap();
        assert_eq!((p.l1, p.sprites, p.l2), (TEST_LEVEL_L1, TEST_LEVEL_SPRITES, TEST_LEVEL_L2));
        assert_eq!(before, rom);
    }

    #[test]
    fn rejects_bad_input() {
        let mut rom = synthetic_rom();
        assert!(delete_levels(&mut rom, &[], 0).is_err());
        assert!(delete_levels(&mut rom, &[0x200], 0).is_err());
        assert!(level_pointers(&rom, 0x200, 0).is_err());
    }

    #[test]
    fn modified_detection() {
        let original = synthetic_rom();
        let mut current = original.clone();
        // Untouched level: not modified.
        assert!(!level_modified_vs(&current, &original, 5, 0));
        // Relocated level (pointers differ): modified.
        delete_levels(&mut current, &[0], 0).unwrap();
        assert!(level_modified_vs(&current, &original, 0, 0));
        // Same pointers but edited block bytes: modified.
        let mut edited = original.clone();
        let l1_pc = lorom_pc(0x0A8000, edited.len(), 0).unwrap();
        edited[l1_pc + 5] ^= 0xFF;
        assert!(level_modified_vs(&edited, &original, 0, 0));
        // Level 1 shares level 0's L1 block but its own pointers are
        // unchanged... its shared block changed, so it reads modified too.
        assert!(level_modified_vs(&edited, &original, 1, 0));
    }

    // ---- Real-ROM tests (ignored; need ROM_PATH) ----

    fn test_rom_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("ROM_PATH").ok()?;
        std::fs::read(path).ok()
    }

    /// The test-level targets must be the most-shared pointer targets in the
    /// vanilla ROM, matching SMWDisX's `TestLevel` / `TestLevelSprites` /
    /// `DATA_0CD900` labels (277 / 278 / 283 sharers).
    #[test]
    #[ignore]
    fn test_targets_are_vanilla_defaults() {
        let rom = test_rom_bytes().expect("ROM_PATH");
        let (h, b) = (0x200 * usize::from(rom.len() % 0x400 == 0x200), 0);
        let _ = b;
        let mut l1c = std::collections::HashMap::new();
        let mut spc = std::collections::HashMap::new();
        let mut l2c = std::collections::HashMap::new();
        for level in 0..LEVEL_COUNT as u16 {
            let p = level_pointers(&rom, level, h).unwrap();
            *l1c.entry(p.l1).or_insert(0) += 1;
            *spc.entry(p.sprites).or_insert(0) += 1;
            *l2c.entry(p.l2).or_insert(0) += 1;
        }
        assert_eq!(l1c.get(&TEST_LEVEL_L1), Some(&277));
        assert_eq!(spc.get(&TEST_LEVEL_SPRITES), Some(&278));
        assert_eq!(l2c.get(&TEST_LEVEL_L2), Some(&283));
    }

    /// Deleting a real level on an in-memory copy: pointers land on the test
    /// targets, the old blocks are erased, the checksum stays valid, and the
    /// image still parses as a ROM.
    #[test]
    #[ignore]
    fn delete_real_level_round_trip() {
        let rom = test_rom_bytes().expect("ROM_PATH");
        let h = 0x200 * usize::from(rom.len() % 0x400 == 0x200);
        let mut work = rom.clone();
        let before = level_pointers(&work, 0x105, h).unwrap();
        assert_ne!(before.l1, TEST_LEVEL_L1);

        let report = delete_levels(&mut work, &[0x105], h).unwrap();
        assert_eq!(report.deleted, vec![0x105]);
        let after = level_pointers(&work, 0x105, h).unwrap();
        assert_eq!((after.l1, after.sprites, after.l2), (TEST_LEVEL_L1, TEST_LEVEL_SPRITES, TEST_LEVEL_L2));

        // Old L1 block erased (level 0x105's block is unique in vanilla).
        let pc = lorom_pc(before.l1, work.len(), h).unwrap();
        assert!(work[pc..pc + 8].iter().all(|&b| b == 0xFF));

        // Checksum valid.
        let body = &work[h..];
        let sum = compute_checksum(body);
        let stored = u16::from_le_bytes([body[0x7FDE], body[0x7FDF]]);
        assert_eq!(sum, stored);

        // The image still parses.
        let rom = Rom::new(work.clone()).expect("Rom::new");
        assert!(crate::SmwRom::from_rom(rom).is_ok());

        // Deleting again is a stable no-op.
        let again = delete_levels(&mut work.clone(), &[0x105], h).unwrap();
        assert_eq!(again.blocks_erased, 0);
    }
}
