//! Share Data Between Levels to Save Space (Lunar Magic 3.50 parity).
//!
//! Verified behavior (FuSoYa's release notes for LM 3.50, 2024-09-24:
//! "an option to share data between levels to save space"): the command scans
//! every level's data, finds levels whose data blocks are byte-identical, and
//! repoints the duplicates at a single shared copy so the freed blocks can be
//! reclaimed as free space.
//!
//! Sharing granularity is one whole data block per level per kind, exactly
//! the unit the game's pointer tables address:
//!
//! - Layer 1: the 5-byte primary header + object data (`$05E000` table)
//! - Sprites: the 1-byte sprite header + sprite data (`$05EC00` table, bank `$07`)
//! - Layer 2: object data with its 5-byte L2 header (`$05E600` table)
//!
//! Whole-block byte equality (header included) is what makes repointing safe:
//! a level repointed at an identical block loads exactly what it loaded
//! before — its header bytes come from the shared copy, which are the same
//! bytes it had. Levels with identical object data but different headers
//! (different music, palette, ...) deliberately do *not* merge.
//!
//! Safety rules (same discipline as the delete-levels implementation):
//! - A block is erased only when no level references it anymore (reference
//!   scan after all repointing), it parses cleanly so its length is known,
//!   and its byte range does not overlap any still-referenced block.
//! - Bank-`$FF` Layer-2 pointers are never touched: they address vanilla
//!   background tilemaps in the original ROM area (283 vanilla slots share
//!   `$FFD900`), and the backgrounds are game data, not level data.
//! - The ROM checksum/complement pair is rewritten.
//!
//! Shared-block save safety: once two levels share a block, the level
//! editor's save path must not erase or overwrite it in place when one of
//! the sharers is saved — see [`block_shared_with_other_levels`], consulted
//! by the save path so a shared level always relocates to fresh space and
//! leaves the shared block intact. (The vanilla ROM already shares blocks
//! this way: 277 test-level slots point at `$068000`.)

use std::collections::{HashMap, HashSet};

use crate::{
    level::{headers::PRIMARY_HEADER_SIZE, ObjectLayer, SpriteLayer, LAYER2_HEADER_SIZE, LEVEL_COUNT},
    rom_expansion::compute_checksum,
    snes_utils::addr::{AddrPc, AddrSnes},
};

// -------------------------------------------------------------------------------------------------
// Pointer tables (SNES addresses)

/// Layer-1 pointer table: `0x200` × 3-byte LoROM SNES address.
const L1_TABLE_SNES: u32 = 0x05E000;
/// Sprite pointer table: `0x200` × 2-byte offset into bank `$07`.
const SPRITE_TABLE_SNES: u32 = 0x05EC00;
/// Layer-2 pointer table: `0x200` × 3-byte LoROM SNES address.
const L2_TABLE_SNES: u32 = 0x05E600;

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

/// A shareable per-level data block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Layer-1 object data: 5-byte primary header + objects (`$05E000`).
    L1,
    /// Sprite data: 1-byte sprite header + sprites (`$05EC00`, bank `$07`).
    Sprites,
    /// Layer-2 object data: 5-byte L2 header + objects (`$05E600`).
    L2,
}

/// What [`share_data_between_levels`] did.
#[derive(Debug, Clone, Default)]
pub struct ShareReport {
    /// Levels whose pointer-table entry was repointed at a shared block.
    pub levels_shared:   usize,
    /// Duplicate groups that were merged (each group = ≥2 levels now sharing
    /// one block).
    pub groups_merged:   usize,
    /// Displaced duplicate blocks erased (`0xFF`-filled).
    pub blocks_erased:   usize,
    /// Bytes erased and thus reclaimed as free space.
    pub bytes_reclaimed: usize,
}

/// One set of levels holding byte-identical data blocks of one kind at more
/// than one distinct address — i.e. one merge
/// [`share_data_between_levels`] would perform.
#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub kind:      BlockKind,
    /// `(level, SNES address)` of every holder, sorted by level number.
    pub holders:   Vec<(u16, u32)>,
    /// Length of the shared block in bytes.
    pub block_len: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ShareDataError {
    #[error("Level {0:03X} out of range (0x000-0x1FF)")]
    LevelOutOfRange(u16),
    #[error("Address conversion failed: {0}")]
    Address(#[from] crate::snes_utils::addr::AddressError),
    #[error("ROM too small for pointer tables")]
    RomTooSmall,
}

// -------------------------------------------------------------------------------------------------

fn table_pc(
    table_snes: u32, entry: usize, entry_size: usize, rom_len: usize, header_offset: usize,
) -> Result<usize, ShareDataError> {
    let base = AddrPc::try_from_lorom(AddrSnes(table_snes))?.as_index() as usize + header_offset;
    let off = base.checked_add(entry * entry_size).ok_or(ShareDataError::RomTooSmall)?;
    if off + entry_size > rom_len {
        return Err(ShareDataError::RomTooSmall);
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
pub fn level_pointers(rom: &[u8], level: u16, header_offset: usize) -> Result<LevelPointers, ShareDataError> {
    if level as usize >= LEVEL_COUNT {
        return Err(ShareDataError::LevelOutOfRange(level));
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

fn table_for(kind: BlockKind) -> (u32, usize) {
    match kind {
        BlockKind::L1 => (L1_TABLE_SNES, 3),
        BlockKind::Sprites => (SPRITE_TABLE_SNES, 2),
        BlockKind::L2 => (L2_TABLE_SNES, 3),
    }
}

/// `true` when the data block `snes` points at is referenced by at least one
/// level other than `level` — i.e. the block is shared. The level editor's
/// save path consults this: a shared block must never be erased or written in
/// place when one of its sharers is saved; the saved level relocates to fresh
/// space instead.
pub fn block_shared_with_other_levels(
    rom: &[u8], kind: BlockKind, snes: u32, level: u16, header_offset: usize,
) -> bool {
    let (table_snes, entry_size) = table_for(kind);
    let mut refs = 0;
    for idx in 0..LEVEL_COUNT {
        let Ok(off) = table_pc(table_snes, idx, entry_size, rom.len(), header_offset) else {
            return false;
        };
        let entry = match kind {
            BlockKind::Sprites => 0x070000 | u16::from_le_bytes([rom[off], rom[off + 1]]) as u32,
            _ => read_u24_le(rom, off),
        };
        if entry == snes {
            refs += 1;
            if refs >= 2 {
                return true;
            }
        }
    }
    let _ = level;
    false
}

/// Byte length of the data block a pointer targets, or `None` when it cannot
/// be measured safely (corrupt data, out of bounds). Bank-`$FF` Layer-2
/// pointers reference vanilla background tilemaps in the original ROM area
/// and are never measured — they are game data, not shareable level data.
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
                return None;
            }
            let start = pc.checked_add(LAYER2_HEADER_SIZE)?;
            let (_, (_, consumed)) = ObjectLayer::parse(rom.get(start..)?).ok()?;
            Some(LAYER2_HEADER_SIZE + consumed)
        }
    }
}

/// The raw bytes of a measured block.
fn block_bytes<'r>(rom: &'r [u8], snes: u32, len: usize, header_offset: usize) -> Option<&'r [u8]> {
    let pc = lorom_pc(snes, rom.len(), header_offset)?;
    rom.get(pc..pc.checked_add(len)?)
}

// -------------------------------------------------------------------------------------------------

/// Rewrite the LoROM checksum/complement pair over the headerless image.
fn rewrite_checksum(body: &mut [u8]) {
    let checksum = compute_checksum(body);
    body[0x7FDC..0x7FDE].copy_from_slice(&(checksum ^ 0xFFFF).to_le_bytes());
    body[0x7FDE..0x7FE0].copy_from_slice(&checksum.to_le_bytes());
}

fn write_table_entry(rom: &mut [u8], kind: BlockKind, level: u16, snes: u32, header_offset: usize) {
    let (table_snes, entry_size) = table_for(kind);
    let idx = level as usize;
    let Ok(off) = table_pc(table_snes, idx, entry_size, rom.len(), header_offset) else {
        return;
    };
    match kind {
        BlockKind::Sprites => {
            let lo = (snes & 0xFFFF) as u16;
            rom[off..off + 2].copy_from_slice(&lo.to_le_bytes());
        }
        _ => {
            rom[off..off + 3].copy_from_slice(&snes.to_le_bytes()[..3]);
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Scan every level's data blocks and return the groups of byte-identical
/// blocks held at more than one distinct address. Non-mutating: this is the
/// analysis half of [`share_data_between_levels`].
pub fn find_duplicate_groups(rom: &[u8], header_offset: usize) -> Result<Vec<DuplicateGroup>, ShareDataError> {
    let mut out = Vec::new();
    for kind in [BlockKind::L1, BlockKind::Sprites, BlockKind::L2] {
        // Group levels by the exact bytes of their data block.
        let mut groups: HashMap<Vec<u8>, Vec<(u16, u32)>> = HashMap::new();
        for level in 0..LEVEL_COUNT as u16 {
            let pointers = level_pointers(rom, level, header_offset)?;
            let snes = match kind {
                BlockKind::L1 => pointers.l1,
                BlockKind::Sprites => pointers.sprites,
                BlockKind::L2 => pointers.l2,
            };
            let Some(len) = block_len(rom, kind, snes, header_offset) else {
                continue;
            };
            let Some(bytes) = block_bytes(rom, snes, len, header_offset) else {
                continue;
            };
            groups.entry(bytes.to_vec()).or_default().push((level, snes));
        }

        for (bytes, mut holders) in groups {
            let mut distinct: Vec<u32> = holders.iter().map(|&(_, snes)| snes).collect();
            distinct.sort_unstable();
            distinct.dedup();
            if distinct.len() < 2 {
                continue;
            }
            holders.sort_by_key(|&(level, _)| level);
            out.push(DuplicateGroup { kind, holders, block_len: bytes.len() });
        }
    }
    // Deterministic order: kind first, then lowest holder level.
    out.sort_by_key(|g| {
        let kind_order = match g.kind {
            BlockKind::L1 => 0,
            BlockKind::Sprites => 1,
            BlockKind::L2 => 2,
        };
        (kind_order, g.holders[0].0)
    });
    Ok(out)
}

/// Share data between levels to save space, Lunar Magic 3.50 style.
///
/// Every level's Layer-1, sprite, and Layer-2 data blocks are compared
/// byte-for-byte; levels holding identical blocks are repointed at the
/// lowest-numbered level's copy, and each displaced block that no level
/// references anymore is erased (`0xFF`-filled), reclaiming it as free space.
///
/// Because sharing is whole-block and byte-identical (headers included), no
/// level loads different data afterwards — sharing is invisible to the game.
///
/// `header_offset` is `0x200` when the image has an SMC copier header, else `0`.
pub fn share_data_between_levels(rom: &mut [u8], header_offset: usize) -> Result<ShareReport, ShareDataError> {
    let mut report = ShareReport::default();
    // Displaced (kind, snes) pairs, for the post-pass reference scan.
    let mut displaced: Vec<(BlockKind, u32)> = Vec::new();

    // Merge each duplicate group: the lowest-numbered level's block is
    // canonical (holders are sorted by level, so it is holders[0]).
    for group in find_duplicate_groups(rom, header_offset)? {
        let canonical = group.holders[0].1;
        let mut merged_levels = 0;
        for &(level, snes) in &group.holders {
            if snes != canonical {
                displaced.push((group.kind, snes));
                write_table_entry(rom, group.kind, level, canonical, header_offset);
                merged_levels += 1;
            }
        }
        if merged_levels > 0 {
            report.levels_shared += merged_levels;
            report.groups_merged += 1;
        }
    }

    if report.groups_merged == 0 {
        return Ok(report);
    }

    // Reference scan: which blocks does any level still point at?
    let mut referenced: HashSet<(BlockKind, u32)> = HashSet::new();
    for level in 0..LEVEL_COUNT as u16 {
        let pointers = level_pointers(rom, level, header_offset)?;
        referenced.insert((BlockKind::L1, pointers.l1));
        referenced.insert((BlockKind::Sprites, pointers.sprites));
        referenced.insert((BlockKind::L2, pointers.l2));
    }
    // Byte ranges of still-referenced measurable blocks, so erasing a
    // displaced duplicate can never clip a live block it overlaps.
    let mut referenced_ranges: Vec<(usize, usize)> = Vec::new();
    for &(kind, snes) in &referenced {
        if let Some(len) = block_len(rom, kind, snes, header_offset) {
            if let Some(pc) = lorom_pc(snes, rom.len(), header_offset) {
                if pc + len <= rom.len() {
                    referenced_ranges.push((pc, pc + len));
                }
            }
        }
    }

    // Erase orphaned, measurable blocks exactly once each.
    let mut erased: HashSet<(BlockKind, u32)> = HashSet::new();
    let mut to_erase: Vec<((BlockKind, u32), usize, usize)> = Vec::new();
    for key @ (kind, snes) in displaced {
        if referenced.contains(&key) || !erased.insert(key) {
            continue;
        }
        let (Some(len), Some(pc)) =
            (block_len(rom, kind, snes, header_offset), lorom_pc(snes, rom.len(), header_offset))
        else {
            continue;
        };
        if pc + len > rom.len() {
            continue;
        }
        if referenced_ranges.iter().any(|&(a, b)| pc < b && a < pc + len) {
            continue;
        }
        to_erase.push((key, pc, len));
    }
    for (_, pc, len) in to_erase {
        rom[pc..pc + len].fill(0xFF);
        report.blocks_erased += 1;
        report.bytes_reclaimed += len;
    }

    rewrite_checksum(&mut rom[header_offset..]);

    Ok(report)
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snes_utils::rom::Rom;

    fn pc_of(rom: &[u8], snes: u32) -> usize {
        lorom_pc(snes, rom.len(), 0).unwrap()
    }

    /// Minimal synthetic ROM: pointer tables at their real SNES addresses
    /// (LoROM-mapped into a 512 KiB image) plus fake data blocks.
    ///
    /// - Level 0: blocks A (L1/sprites/L2).
    /// - Level 1: byte-identical copies A' of A's L1 + sprite blocks at
    ///   different addresses (L2 already shares A's block — no merge needed).
    /// - Level 2: unique blocks B.
    /// - Level 3: corrupt (unparseable) L1 block — must be left alone.
    /// - Rest: point at level 0's blocks (already shared — no merge needed).
    fn synthetic_rom() -> Vec<u8> {
        let mut rom = vec![0xFFu8; 0x80000];

        // Block set A.
        let l1_a: u32 = 0x0A8000;
        let sp_a: u32 = 0x07C000;
        let l2_a: u32 = 0x0B8000;
        // Block set A' (byte-identical copies of A's L1 + sprites).
        let l1_ap: u32 = 0x0C8000;
        let sp_ap: u32 = 0x07D000;
        // Block set B (unique).
        let l1_b: u32 = 0x0D8000;
        let sp_b: u32 = 0x07C800;
        let l2_b: u32 = 0x0E8000;
        // Corrupt L1 block (no terminator in range / bad data).
        let l1_bad: u32 = 0x0F8000;

        let write_l1 = |rom: &mut Vec<u8>, snes: u32, hdr: [u8; 5], body: &[u8]| {
            let pc = pc_of(rom, snes);
            rom[pc..pc + 5].copy_from_slice(&hdr);
            rom[pc + 5..pc + 5 + body.len()].copy_from_slice(body);
        };
        let write_sp = |rom: &mut Vec<u8>, snes: u32, hdr: u8, body: &[u8]| {
            let pc = pc_of(rom, snes);
            rom[pc] = hdr;
            rom[pc + 1..pc + 1 + body.len()].copy_from_slice(body);
        };
        let l1_body = [0x2F, 0x00, 0x00, 0xFF];
        let sp_body = [0x01, 0x02, 0x03, 0xFF];
        let l2_body = [0x40, 0x00, 0xFF];
        write_l1(&mut rom, l1_a, [1, 2, 3, 4, 5], &l1_body);
        write_l1(&mut rom, l1_ap, [1, 2, 3, 4, 5], &l1_body);
        write_l1(&mut rom, l1_b, [5, 4, 3, 2, 1], &[0x2F, 0x10, 0x00, 0xFF]);
        write_sp(&mut rom, sp_a, 0x11, &sp_body);
        write_sp(&mut rom, sp_ap, 0x11, &sp_body);
        write_sp(&mut rom, sp_b, 0x12, &[0x09, 0x08, 0x07, 0xFF]);
        write_l1(&mut rom, l2_a, [9, 9, 9, 9, 9], &l2_body);
        write_l1(&mut rom, l2_b, [8, 8, 8, 8, 8], &[0x41, 0x00, 0xFF]);
        // Corrupt: no 0xFF terminator within a sane range (filled with 0x2F).
        let bad_pc = pc_of(&rom, l1_bad);
        rom[bad_pc..bad_pc + 5].copy_from_slice(&[1, 1, 1, 1, 1]);
        rom[bad_pc + 5..bad_pc + 205].fill(0x2F);

        let set = |rom: &mut Vec<u8>, level: usize, l1: u32, sp: u32, l2: u32| {
            let o = pc_of(rom, L1_TABLE_SNES) + level * 3;
            rom[o..o + 3].copy_from_slice(&l1.to_le_bytes()[..3]);
            let o = pc_of(rom, SPRITE_TABLE_SNES) + level * 2;
            rom[o..o + 2].copy_from_slice(&((sp & 0xFFFF) as u16).to_le_bytes());
            let o = pc_of(rom, L2_TABLE_SNES) + level * 3;
            rom[o..o + 3].copy_from_slice(&l2.to_le_bytes()[..3]);
        };
        for level in 0..LEVEL_COUNT {
            set(&mut rom, level, l1_a, sp_a, l2_a);
        }
        set(&mut rom, 1, l1_ap, sp_ap, l2_a);
        set(&mut rom, 2, l1_b, sp_b, l2_b);
        set(&mut rom, 3, l1_bad, sp_b, l2_b);

        rewrite_checksum(&mut rom);
        rom
    }

    #[test]
    fn merges_identical_blocks_and_reclaims_the_copies() {
        let mut rom = synthetic_rom();
        let report = share_data_between_levels(&mut rom, 0).unwrap();

        // Level 1's L1 + sprite pointers now match level 0's.
        let p0 = level_pointers(&rom, 0, 0).unwrap();
        let p1 = level_pointers(&rom, 1, 0).unwrap();
        assert_eq!(p1.l1, p0.l1);
        assert_eq!(p1.sprites, p0.sprites);
        // L2 was already shared; unique blocks untouched.
        let p2 = level_pointers(&rom, 2, 0).unwrap();
        assert_eq!(p2.l1, 0x0D8000);

        assert_eq!(report.levels_shared, 2);
        assert_eq!(report.groups_merged, 2);

        // The displaced copies were erased...
        let ap_pc = pc_of(&rom, 0x0C8000);
        assert!(rom[ap_pc..ap_pc + 9].iter().all(|&b| b == 0xFF));
        let spap_pc = pc_of(&rom, 0x07D000);
        assert!(rom[spap_pc..spap_pc + 5].iter().all(|&b| b == 0xFF));
        // ...but the canonical blocks survived (still referenced).
        let a_pc = pc_of(&rom, 0x0A8000);
        assert_eq!(&rom[a_pc..a_pc + 5], &[1, 2, 3, 4, 5]);
        assert!(report.bytes_reclaimed > 0);
        assert_eq!(report.blocks_erased, 2);

        // Corrupt block untouched.
        let bad_pc = pc_of(&rom, 0x0F8000);
        assert_eq!(rom[bad_pc + 5], 0x2F);

        // Checksum still valid.
        let sum = compute_checksum(&rom);
        let stored = u16::from_le_bytes([rom[0x7FDE], rom[0x7FDF]]);
        assert_eq!(sum, stored);
    }

    #[test]
    fn already_shared_blocks_are_not_remerged() {
        // Every level points at the same blocks: nothing to do.
        let mut only_shared = vec![0xFFu8; 0x80000];
        let l1_a: u32 = 0x0A8000;
        let sp_a: u32 = 0x07C000;
        let l2_a: u32 = 0x0B8000;
        let pc = pc_of(&only_shared, l1_a);
        only_shared[pc..pc + 5].copy_from_slice(&[1, 2, 3, 4, 5]);
        only_shared[pc + 5..pc + 9].copy_from_slice(&[0x2F, 0x00, 0x00, 0xFF]);
        let pc = pc_of(&only_shared, sp_a);
        only_shared[pc] = 0x11;
        only_shared[pc + 1..pc + 5].copy_from_slice(&[0x01, 0x02, 0x03, 0xFF]);
        let pc = pc_of(&only_shared, l2_a);
        only_shared[pc..pc + 5].copy_from_slice(&[9, 9, 9, 9, 9]);
        only_shared[pc + 5..pc + 8].copy_from_slice(&[0x40, 0x00, 0xFF]);
        for level in 0..LEVEL_COUNT {
            let o = pc_of(&only_shared, L1_TABLE_SNES) + level * 3;
            only_shared[o..o + 3].copy_from_slice(&l1_a.to_le_bytes()[..3]);
            let o = pc_of(&only_shared, SPRITE_TABLE_SNES) + level * 2;
            only_shared[o..o + 2].copy_from_slice(&((sp_a & 0xFFFF) as u16).to_le_bytes());
            let o = pc_of(&only_shared, L2_TABLE_SNES) + level * 3;
            only_shared[o..o + 3].copy_from_slice(&l2_a.to_le_bytes()[..3]);
        }
        rewrite_checksum(&mut only_shared);
        let before = only_shared.clone();
        let report = share_data_between_levels(&mut only_shared, 0).unwrap();
        assert_eq!(report.groups_merged, 0);
        assert_eq!(report.levels_shared, 0);
        assert_eq!(report.blocks_erased, 0);
        assert_eq!(before, only_shared, "no-op run must not touch any byte");
    }

    #[test]
    fn rejects_out_of_range_level() {
        let rom = synthetic_rom();
        assert!(level_pointers(&rom, 0x200, 0).is_err());
    }

    #[test]
    fn shared_detection_counts_other_levels() {
        let rom = synthetic_rom();
        // Level 0's L1 block is pointed at by levels 0, 4, 5, ... (all but
        // 1, 2, 3) -> shared.
        assert!(block_shared_with_other_levels(&rom, BlockKind::L1, 0x0A8000, 0, 0));
        // Level 2's L1 block is pointed at only by level 2 (and level 3's
        // corrupt pointer is a different address) -> not shared.
        assert!(!block_shared_with_other_levels(&rom, BlockKind::L1, 0x0D8000, 2, 0));
        // Level 1's copy is unique to level 1 -> not shared.
        assert!(!block_shared_with_other_levels(&rom, BlockKind::L1, 0x0C8000, 1, 0));
    }

    // ---- Real-ROM tests (ignored; need ROM_PATH) ----

    fn test_rom_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("ROM_PATH").ok()?;
        std::fs::read(path).ok()
    }

    /// Sharing must be invisible to the game: every level's decoded data is
    /// identical before and after, the image still parses, and the checksum
    /// stays valid. A second run must be a stable no-op.
    #[test]
    #[ignore]
    fn share_preserves_every_level_on_real_rom() {
        let rom = test_rom_bytes().expect("ROM_PATH");
        let h = 0x200 * usize::from(rom.len() % 0x400 == 0x200);

        fn level_data(rom: &[u8], h: usize) -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
            let mut out = Vec::new();
            for level in 0..LEVEL_COUNT as u16 {
                let p = level_pointers(rom, level, h).unwrap();
                let grab = |kind: BlockKind, snes: u32| -> Vec<u8> {
                    match block_len(rom, kind, snes, h).and_then(|len| block_bytes(rom, snes, len, h)) {
                        Some(b) => b.to_vec(),
                        None => Vec::new(),
                    }
                };
                out.push((grab(BlockKind::L1, p.l1), grab(BlockKind::Sprites, p.sprites), grab(BlockKind::L2, p.l2)));
            }
            out
        }

        let before = level_data(&rom, h);
        let mut work = rom.clone();
        let report = share_data_between_levels(&mut work, h).unwrap();
        let after = level_data(&work, h);
        assert_eq!(before, after, "every level must load byte-identical data after sharing");

        // Checksum valid over the headerless body.
        let body = &work[h..];
        let sum = compute_checksum(body);
        let stored = u16::from_le_bytes([body[0x7FDE], body[0x7FDF]]);
        assert_eq!(sum, stored);

        // The image still parses as a ROM.
        let parsed = Rom::new(work.clone()).expect("Rom::new");
        assert!(crate::SmwRom::from_rom(parsed).is_ok());

        // Second run: stable no-op.
        let snapshot = work.clone();
        let report2 = share_data_between_levels(&mut work, h).unwrap();
        assert_eq!(report2.groups_merged, 0);
        assert_eq!(snapshot, work);
        let _ = report;
    }

    /// Byte ranges of every measurable level-data block in the image.
    fn measured_ranges(rom: &[u8], h: usize) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        for level in 0..LEVEL_COUNT as u16 {
            let p = level_pointers(rom, level, h).unwrap();
            for (kind, snes) in [(BlockKind::L1, p.l1), (BlockKind::Sprites, p.sprites), (BlockKind::L2, p.l2)] {
                if let Some(len) = block_len(rom, kind, snes, h) {
                    if let Some(pc) = lorom_pc(snes, rom.len(), h) {
                        if pc + len <= rom.len() {
                            ranges.push((pc, pc + len));
                        }
                    }
                }
            }
        }
        ranges
    }

    /// Find `len` free bytes in `[start, end)` (headerless PC space) that are
    /// both `0xFF`-filled and disjoint from every measured block. Plain
    /// `0xFF`-run detection is not enough: sprite data can contain interior
    /// `0xFF` bytes inside a live block's measured range.
    fn find_plant_spot(rom: &[u8], len: usize, start: usize, end: usize, h: usize) -> Option<usize> {
        let ranges = measured_ranges(rom, h);
        let mut pc = start;
        while pc + len <= end {
            let file = pc + h;
            if rom[file..file + len].iter().all(|&b| b == 0xFF)
                && !ranges.iter().any(|&(a, b)| file < b && a < file + len)
            {
                return Some(file);
            }
            pc += 1;
        }
        None
    }

    /// Artificial duplicates of a real level's blocks get merged back and the
    /// copies' bytes reclaimed.
    #[test]
    #[ignore]
    fn share_merges_artificial_duplicates_on_real_rom() {
        let rom = test_rom_bytes().expect("ROM_PATH");
        let h = 0x200 * usize::from(rom.len() % 0x400 == 0x200);
        let mut work = rom.clone();

        // Duplicate level 0x100's measurable blocks into free space and point
        // levels 0x104/0x105 at the copies. (Source is the lowest-numbered
        // holder, so its blocks are the canonical survivors — unless some
        // even lower level already holds identical bytes, which the
        // assertions below tolerate.)
        let src_level: u16 = 0x100;
        let src = level_pointers(&work, src_level, h).unwrap();
        // (kind, original snes, copy snes, block len, copy pc, original pc)
        let mut copies: Vec<(BlockKind, u32, u32, usize, usize, usize)> = Vec::new();
        for kind in [BlockKind::L1, BlockKind::Sprites, BlockKind::L2] {
            let snes = match kind {
                BlockKind::L1 => src.l1,
                BlockKind::Sprites => src.sprites,
                BlockKind::L2 => src.l2,
            };
            let Some(len) = block_len(&work, kind, snes, h) else { continue };
            let bytes = block_bytes(&work, snes, len, h).unwrap().to_vec();
            // Plant the copy where it cannot disturb any live block (see
            // `find_plant_spot`).
            let pc = match kind {
                BlockKind::Sprites => find_plant_spot(&work, len, 0x38000, 0x40000, h).expect("bank $07 plant spot"),
                _ => find_plant_spot(&work, len, 0x008000, work.len() - h, h).expect("plant spot"),
            };
            work[pc..pc + len].copy_from_slice(&bytes);
            let copy_snes = match kind {
                BlockKind::Sprites => 0x070000 | (0x8000 + (pc - 0x038000 - h)) as u32,
                _ => {
                    let rel = (pc - h) as u32;
                    0x8000 + rel % 0x8000 + ((rel / 0x8000) << 16)
                }
            };
            let orig_pc = lorom_pc(snes, work.len(), h).unwrap();
            copies.push((kind, snes, copy_snes, len, pc, orig_pc));
        }
        assert!(!copies.is_empty(), "level 0x100 should have measurable blocks");
        for &(kind, _, copy_snes, _, _, _) in &copies {
            for &level in &[0x104u16, 0x105] {
                write_table_entry(&mut work, kind, level, copy_snes, h);
            }
        }
        let dup_bytes: usize = copies.iter().map(|&(_, _, _, len, _, _)| len).sum();

        fn level_data(rom: &[u8], h: usize) -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
            let mut out = Vec::new();
            for level in 0..LEVEL_COUNT as u16 {
                let p = level_pointers(rom, level, h).unwrap();
                let grab = |kind: BlockKind, snes: u32| -> Vec<u8> {
                    match block_len(rom, kind, snes, h).and_then(|len| block_bytes(rom, snes, len, h)) {
                        Some(b) => b.to_vec(),
                        None => Vec::new(),
                    }
                };
                out.push((grab(BlockKind::L1, p.l1), grab(BlockKind::Sprites, p.sprites), grab(BlockKind::L2, p.l2)));
            }
            out
        }
        let before = level_data(&work, h);

        let report = share_data_between_levels(&mut work, h).unwrap();
        assert!(report.groups_merged >= copies.len());
        assert!(report.bytes_reclaimed >= dup_bytes);

        // All holders of each duplicated block converge on one address.
        let p_src = level_pointers(&work, src_level, h).unwrap();
        for &level in &[0x104u16, 0x105] {
            let p = level_pointers(&work, level, h).unwrap();
            for &(kind, _, _, _, _, _) in &copies {
                match kind {
                    BlockKind::L1 => assert_eq!(p.l1, p_src.l1, "level {level:03X} L1 converges"),
                    BlockKind::Sprites => assert_eq!(p.sprites, p_src.sprites, "level {level:03X} sprites converge"),
                    BlockKind::L2 => assert_eq!(p.l2, p_src.l2, "level {level:03X} L2 converges"),
                }
            }
        }
        // For each duplicated block, at least one of {copy, original} was
        // erased over its full length (the other is the canonical survivor).
        for &(kind, _, copy_snes, len, pc_copy, pc_orig) in &copies {
            let copy_erased = work[pc_copy..pc_copy + len].iter().all(|&b| b == 0xFF);
            let orig_erased = work[pc_orig..pc_orig + len].iter().all(|&b| b == 0xFF);
            assert!(copy_erased || orig_erased, "{kind:?}: copy {copy_snes:06X} or original reclaimed");
        }

        // Every level loads byte-identical data after sharing; checksum valid.
        assert_eq!(before, level_data(&work, h), "sharing is invisible to the game");
        let body = &work[h..];
        let sum = compute_checksum(body);
        assert_eq!(sum, u16::from_le_bytes([body[0x7FDE], body[0x7FDF]]));
    }
}
