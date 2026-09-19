//! Secret Exit 2 / 3 direction-to-enable settings (Lunar Magic v3.00 parity).
//!
//! The official LM v3.00 changelog says three things about this feature:
//!
//! - "made it possible to use Secret Exit 2 and Secret Exit 3 in the game"
//! - "the overworld editor has gained direction to enable settings for both"
//! - "the Secret Exit 2 and Secret Exit 3 goal point tape sprites have been
//!   added to the sprite list"
//!
//! What the vanilla ROM gives us (verified against SMWDisX, `bank_00.asm`
//! `InitGoalTape` / `CODE_00C9C2`): a goal tape's exit mode is selected by the
//! sprite's extra bits — `SecretGoalTape = extra_bits`, then `OWLevelExitMode
//! = SecretGoalTape + 1`. Extra bits 0/1 select exit modes 1/2 (normal exit /
//! secret exit 1). Extra bits 2/3 hit vanilla's quirky paths (mode 3 lands in
//! a half-removed debug branch that also rewrites `OWPlayerSubmap`; mode 4
//! maps through `DATA_049060` to a benign auto-walk direction), so real
//! Secret Exit 2/3 *behavior* in-game needs LM v3.00's ASM — which this
//! editor does not install.
//!
//! What the vanilla ROM does *not* have: a stored per-level
//! "direction to enable" table for secret exits 2/3. In vanilla, overworld
//! movement directions are granted by event activation and runtime state
//! (`InitLevelTileMovementData`, `DATA_04941E` in `bank_04.asm`); there is no
//! per-exit direction table to edit. LM v3.00 keeps these settings in its own
//! space, whose exact on-disk format is not publicly documented — so
//! smw-editor stores them in its own RATS block instead:
//!
//! ```text
//! [STAR tag: "STAR" + u16 size + u16 ~size]
//! [payload: "SMWSEXIT" (8) | version u8 (=1) | count u8 | entries...]
//! [entry: level u16 LE | exit2_dirs u8 | exit3_dirs u8]
//! ```
//!
//! Direction bits use the vanilla `OWLevelTileSettings` convention from
//! `bank_04.asm` (`$28,$03` = "enable left/right", `$4D,$01` = "enable right",
//! `$5C,$02` = "enable left", `$57,$04` = "enable down", `$5B,$08` =
//! "enable up"): bit0 `$01` = right, bit1 `$02` = left, bit2 `$04` = down,
//! bit3 `$08` = up. Only the low nibble is stored; high bits are masked off
//! on decode.
//!
//! There is no vanilla pointer to repoint for this data, so the block is
//! found by scanning the ROM for its RATS tag + magic. On save, every
//! matching block is erased first (so repeated saves never accumulate
//! orphans) and a fresh block is written into free space; an empty settings
//! set erases the block and writes nothing. A ROM the user never touched
//! keeps no block at all.

use crate::{
    freespace::find_free_space,
    snes_utils::addr::{AddrPc, AddrSnes},
};

/// Magic at the start of our secret-exit RATS payload.
const SECRET_EXIT_MAGIC: &[u8; 8] = b"SMWSEXIT";
const SECRET_EXIT_VERSION: u8 = 1;
/// Payload header: magic + version + entry count.
const SECRET_EXIT_HEADER_LEN: usize = 8 + 1 + 1;
/// On-disk size of one entry: level u16 LE + exit2 mask + exit3 mask.
const ENTRY_LEN: usize = 4;

/// Vanilla `OWLevelTileSettings` direction bits (`bank_04.asm`).
pub const DIR_RIGHT: u8 = 0x01;
pub const DIR_LEFT: u8 = 0x02;
pub const DIR_DOWN: u8 = 0x04;
pub const DIR_UP: u8 = 0x08;
/// Mask of the stored direction bits (low nibble).
pub const DIR_MASK: u8 = 0x0F;

/// Highest level number addressable (vanilla level numbers are 9-bit).
pub const MAX_LEVEL: u16 = 0x1FF;

/// Direction-to-enable settings for one level's Secret Exit 2 and Secret
/// Exit 3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecretExitEntry {
    /// Level number (`0..=0x1FF`).
    pub level: u16,
    /// Directions to enable on Secret Exit 2 ([`DIR_RIGHT`] etc.).
    pub exit2: u8,
    /// Directions to enable on Secret Exit 3.
    pub exit3: u8,
}

/// The full per-level Secret Exit 2/3 settings table. Entries are kept
/// sorted by level and unique per level.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecretExitSettings {
    pub entries: Vec<SecretExitEntry>,
}

impl SecretExitSettings {
    /// Settings for `level`, if any.
    pub fn get(&self, level: u16) -> Option<SecretExitEntry> {
        self.entries.iter().find(|e| e.level == level).copied()
    }

    /// Insert or replace the entry for `level`. A zero/zero mask pair
    /// removes the entry instead of storing a no-op.
    pub fn set(&mut self, entry: SecretExitEntry) {
        self.remove(entry.level);
        if entry.exit2 & DIR_MASK != 0 || entry.exit3 & DIR_MASK != 0 {
            let mut e = entry;
            e.level = e.level.min(MAX_LEVEL);
            e.exit2 &= DIR_MASK;
            e.exit3 &= DIR_MASK;
            self.entries.push(e);
            self.entries.sort_by_key(|e| e.level);
        }
    }

    /// Remove the entry for `level`.
    pub fn remove(&mut self, level: u16) {
        self.entries.retain(|e| e.level != level);
    }

    /// Encode to the RATS payload format (without the `STAR` tag).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SECRET_EXIT_HEADER_LEN + self.entries.len() * ENTRY_LEN);
        out.extend_from_slice(SECRET_EXIT_MAGIC);
        out.push(SECRET_EXIT_VERSION);
        out.push(self.entries.len().min(u8::MAX as usize) as u8);
        for e in self.entries.iter().take(u8::MAX as usize) {
            out.extend_from_slice(&e.level.to_le_bytes());
            out.push(e.exit2 & DIR_MASK);
            out.push(e.exit3 & DIR_MASK);
        }
        out
    }

    /// Decode a RATS payload produced by [`Self::encode_payload`].
    pub fn decode_payload(payload: &[u8]) -> Result<Self, SecretExitError> {
        let corrupt = |msg: &str| SecretExitError::Corrupt(msg.to_string());
        if payload.len() < SECRET_EXIT_HEADER_LEN {
            return Err(corrupt("payload shorter than header"));
        }
        if &payload[..8] != SECRET_EXIT_MAGIC {
            return Err(corrupt("bad magic"));
        }
        if payload[8] != SECRET_EXIT_VERSION {
            return Err(SecretExitError::Corrupt(format!("unsupported version {}", payload[8])));
        }
        let count = payload[9] as usize;
        let mut entries = Vec::with_capacity(count);
        let mut pos = SECRET_EXIT_HEADER_LEN;
        for _ in 0..count {
            let bytes = payload.get(pos..pos + ENTRY_LEN).ok_or_else(|| corrupt("entry overruns payload"))?;
            entries.push(SecretExitEntry {
                level: u16::from_le_bytes([bytes[0], bytes[1]]).min(MAX_LEVEL),
                exit2: bytes[2] & DIR_MASK,
                exit3: bytes[3] & DIR_MASK,
            });
            pos += ENTRY_LEN;
        }
        entries.sort_by_key(|e| e.level);
        entries.dedup_by_key(|e| e.level);
        Ok(Self { entries })
    }
}

/// Errors from [`write_secret_exits`].
#[derive(Debug, thiserror::Error)]
pub enum SecretExitError {
    #[error("no ROM free space for {0} bytes of secret-exit settings")]
    NoFreeSpace(usize),
    #[error("secret-exit settings payload too large ({0} bytes)")]
    PayloadTooLarge(usize),
    #[error("corrupt secret-exit settings block: {0}")]
    Corrupt(String),
}

/// Validate a RATS tag at `file_off`; returns the payload range on success.
fn rats_payload_range(rom_bytes: &[u8], file_off: usize) -> Option<std::ops::Range<usize>> {
    let tag = rom_bytes.get(file_off..file_off + 8)?;
    if &tag[..4] != b"STAR" {
        return None;
    }
    let size = u16::from_le_bytes([tag[4], tag[5]]) as usize;
    let inv = u16::from_le_bytes([tag[6], tag[7]]);
    if size as u16 ^ inv != 0xFFFF {
        return None;
    }
    let start = file_off + 8;
    let end = start + size + 1;
    if end > rom_bytes.len() {
        return None;
    }
    Some(start..end)
}

/// File offsets of every smw-editor secret-exit RATS block in the ROM.
fn find_blocks(rom_bytes: &[u8], header_offset: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let start = header_offset;
    let mut off = start;
    while off + 8 <= rom_bytes.len() {
        if &rom_bytes[off..off + 4] == b"STAR" {
            if let Some(range) = rats_payload_range(rom_bytes, off) {
                if range.len() >= SECRET_EXIT_HEADER_LEN
                    && rom_bytes[range.start..range.start + 8] == *SECRET_EXIT_MAGIC
                {
                    out.push(off);
                }
                off = range.end;
                continue;
            }
        }
        off += 1;
    }
    out
}

/// Parse the secret-exit settings. Returns the last decodable block (the
/// most recently written one); empty settings when the ROM has no block.
pub fn parse_secret_exits(rom_bytes: &[u8], header_offset: usize) -> SecretExitSettings {
    let mut settings = SecretExitSettings::default();
    for off in find_blocks(rom_bytes, header_offset) {
        if let Some(range) = rats_payload_range(rom_bytes, off) {
            if let Ok(parsed) = SecretExitSettings::decode_payload(&rom_bytes[range]) {
                settings = parsed;
            }
        }
    }
    settings
}

/// Write the secret-exit settings: erase every smw-editor-authored block,
/// then allocate fresh RATS-tagged free space for the new payload. An empty
/// settings set erases the block(s) and writes nothing.
pub fn write_secret_exits(
    settings: &SecretExitSettings, rom_bytes: &mut [u8], header_offset: usize,
) -> Result<(), SecretExitError> {
    for off in find_blocks(rom_bytes, header_offset) {
        if let Some(range) = rats_payload_range(rom_bytes, off) {
            rom_bytes[off..range.end].fill(0xFF);
        }
    }
    if settings.entries.is_empty() {
        return Ok(());
    }
    let payload = settings.encode_payload();
    if payload.len() > 0x10000 {
        return Err(SecretExitError::PayloadTooLarge(payload.len()));
    }
    let total = 8 + payload.len(); // RATS tag + payload
    let pc = find_free_space(rom_bytes, total, 0x008000, header_offset).ok_or(SecretExitError::NoFreeSpace(total))?;
    let file_off = pc + header_offset;
    let size_field = (payload.len() - 1) as u16;
    rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
    rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
    rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
    rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);
    // Sanity: the block must decode back to what we wrote.
    let range = rats_payload_range(rom_bytes, file_off)
        .ok_or_else(|| SecretExitError::Corrupt("just-written tag unreadable".into()))?;
    let back = SecretExitSettings::decode_payload(&rom_bytes[range])
        .map_err(|e| SecretExitError::Corrupt(format!("round-trip failed: {e}")))?;
    if back != *settings {
        return Err(SecretExitError::Corrupt("round-trip mismatch".into()));
    }
    let _ = AddrSnes::try_from_lorom(AddrPc(pc as u32));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rom() -> Vec<u8> {
        vec![0xFFu8; 0x80000]
    }

    fn sample() -> SecretExitSettings {
        let mut s = SecretExitSettings::default();
        s.set(SecretExitEntry { level: 0x101, exit2: DIR_UP | DIR_RIGHT, exit3: DIR_LEFT });
        s.set(SecretExitEntry { level: 0x007, exit2: DIR_DOWN, exit3: 0 });
        s
    }

    #[test]
    fn payload_round_trip() {
        let s = sample();
        let back = SecretExitSettings::decode_payload(&s.encode_payload()).unwrap();
        assert_eq!(back, s);
        // Entries stay sorted by level.
        assert_eq!(back.entries[0].level, 0x007);
        assert_eq!(back.entries[1].level, 0x101);
    }

    #[test]
    fn set_replaces_and_zero_removes() {
        let mut s = SecretExitSettings::default();
        s.set(SecretExitEntry { level: 5, exit2: DIR_UP, exit3: 0 });
        s.set(SecretExitEntry { level: 5, exit2: DIR_DOWN, exit3: DIR_LEFT });
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.get(5).unwrap().exit2, DIR_DOWN);
        s.set(SecretExitEntry { level: 5, exit2: 0, exit3: 0 });
        assert!(s.get(5).is_none());
    }

    #[test]
    fn decode_rejects_bad_magic_version_and_truncation() {
        let mut payload = sample().encode_payload();
        payload[0] = b'X';
        assert!(SecretExitSettings::decode_payload(&payload).is_err());
        let mut payload = sample().encode_payload();
        payload[8] = 99;
        assert!(matches!(SecretExitSettings::decode_payload(&payload), Err(SecretExitError::Corrupt(_))));
        let payload = sample().encode_payload();
        assert!(SecretExitSettings::decode_payload(&payload[..payload.len() - 1]).is_err());
        assert!(SecretExitSettings::decode_payload(&[]).is_err());
    }

    #[test]
    fn write_then_parse_round_trip_on_synthetic_rom() {
        let mut rom = test_rom();
        write_secret_exits(&sample(), &mut rom, 0).unwrap();
        assert_eq!(parse_secret_exits(&rom, 0), sample());
    }

    #[test]
    fn write_empty_erases_block() {
        let mut rom = test_rom();
        write_secret_exits(&sample(), &mut rom, 0).unwrap();
        assert!(!find_blocks(&rom, 0).is_empty());
        write_secret_exits(&SecretExitSettings::default(), &mut rom, 0).unwrap();
        assert!(find_blocks(&rom, 0).is_empty());
        assert_eq!(parse_secret_exits(&rom, 0), SecretExitSettings::default());
    }

    #[test]
    fn repeated_writes_leave_no_orphans_and_latest_wins() {
        let mut rom = test_rom();
        write_secret_exits(&sample(), &mut rom, 0).unwrap();
        let mut updated = sample();
        updated.set(SecretExitEntry { level: 0x101, exit2: DIR_LEFT, exit3: DIR_UP });
        write_secret_exits(&updated, &mut rom, 0).unwrap();
        assert_eq!(find_blocks(&rom, 0).len(), 1, "repeated saves must not orphan blocks");
        assert_eq!(parse_secret_exits(&rom, 0), updated);
    }

    #[test]
    fn direction_bit_convention() {
        // Matches the vanilla OWLevelTileSettings bits from bank_04.asm.
        assert_eq!(DIR_RIGHT, 0x01);
        assert_eq!(DIR_LEFT, 0x02);
        assert_eq!(DIR_DOWN, 0x04);
        assert_eq!(DIR_UP, 0x08);
    }

    /// Real-ROM validation: a vanilla SMW ROM carries no `SMWSEXIT` block, so
    /// parsing must yield empty settings (and, importantly, the tag scan must
    /// not false-positive on real ROM data). Requires `ROM_PATH`. Read-only.
    #[test]
    #[ignore]
    fn real_rom_vanilla_has_no_secret_exit_block() {
        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must be set");
        let raw = std::fs::read(&rom_path).expect("read rom");
        let (bytes, header_offset) = if raw.len() % 0x400 == 0x200 { (raw[0x200..].to_vec(), 0x200) } else { (raw, 0) };
        assert!(find_blocks(&bytes, header_offset).is_empty());
        assert_eq!(parse_secret_exits(&bytes, header_offset), SecretExitSettings::default());
    }
}
