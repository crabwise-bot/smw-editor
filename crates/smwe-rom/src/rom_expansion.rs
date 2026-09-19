//! ROM expansion — Lunar Magic's "Expand ROM".
//!
//! Grows a LoROM image to a larger power-of-two size (1 MB / 2 MB / 4 MB) by
//! appending `0xFF`-filled banks, then fixes the internal header: the ROM size
//! byte and the checksum/complement pair are rewritten.
//!
//! The appended space is `0xFF` fill, so
//! [`crate::freespace::find_free_space`] picks it up automatically — every
//! write path that repoints data (level layer/sprite data, GFX files, message
//! boxes, overworld layer 2, ...) gets the new space with no further changes.
//!
//! Checksum convention: the 16-bit wrapping sum of every byte of the image,
//! *excluding* the 4 complement/checksum bytes themselves
//! (`0x7FDC..0x7FE0`), with the complement stored as `checksum ^ 0xFFFF`.
//! This is self-consistent (recomputing over an expanded image reproduces the
//! stored value); the header-detection heuristic only relies on the pair
//! being complementary, which this preserves.

use std::sync::Arc;

use thiserror::Error;

use crate::{
    internal_header::InternalHeaderParseError,
    snes_utils::rom::{Rom, SMC_HEADER_SIZE},
};

// -------------------------------------------------------------------------------------------------

/// LoROM expansion targets in bytes, mirroring Lunar Magic (LoROM is capped at
/// 4 MB / 32 Mbit — larger needs ExLoROM, which LM handles separately).
pub const EXPANSION_TARGETS: &[usize] = &[0x10_0000, 0x20_0000, 0x40_0000];

/// Largest LoROM image this module will produce.
pub const MAX_LOROM_SIZE: usize = 0x40_0000;

/// LoROM internal-header PC offsets of the fields expansion rewrites.
const HEADER_BASE: usize = 0x7FC0;
const ROM_SIZE_OFFSET: usize = HEADER_BASE + 0x17; // 0x7FD7
const COMPLEMENT_OFFSET: usize = HEADER_BASE + 0x1C; // 0x7FDC
const CHECKSUM_OFFSET: usize = HEADER_BASE + 0x1E; // 0x7FDE

// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ExpansionError {
    #[error("ROM is {0:#x} bytes; only LoROM images of 512 KB, 1 MB, or 2 MB can be expanded")]
    UnsupportedSize(usize),
    #[error("target size {0:#x} is not a supported LoROM expansion size (1 MB, 2 MB, 4 MB)")]
    UnsupportedTarget(usize),
    #[error("target size {0:#x} must be larger than the current size {1:#x}")]
    TargetNotLarger(usize, usize),
    #[error("expanded ROM failed to re-parse its internal header: {0}")]
    Header(#[from] InternalHeaderParseError),
}

// -------------------------------------------------------------------------------------------------

/// Sizes (in bytes) a ROM of `current_size` bytes can still expand to,
/// ascending. Empty when the ROM is already at (or past) the LoROM cap.
pub fn expansion_targets(current_size: usize) -> Vec<usize> {
    EXPANSION_TARGETS.iter().copied().filter(|&t| t > current_size).collect()
}

/// Human-readable size, e.g. `512 KB` / `4 MB`.
pub fn format_size(size: usize) -> String {
    if size % 0x10_0000 == 0 {
        format!("{} MB", size / 0x10_0000)
    } else {
        format!("{} KB", size / 0x400)
    }
}

/// SNES 16-bit checksum: wrapping sum of every byte of the (headerless) image,
/// excluding the 4 complement/checksum bytes at `0x7FDC..0x7FE0`.
pub fn compute_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    for (i, &b) in bytes.iter().enumerate() {
        if !(COMPLEMENT_OFFSET..COMPLEMENT_OFFSET + 4).contains(&i) {
            sum = sum.wrapping_add(b as u32);
        }
    }
    (sum & 0xFFFF) as u16
}

/// Rewrite the internal-header checksum/complement pair in place over
/// `bytes` (the headerless image). Same math as [`expand_rom`], factored out
/// so other ROM-surgery operations (e.g. level deletion) can repair the
/// checksum without expanding.
pub fn rewrite_checksum(bytes: &mut [u8]) {
    let checksum = compute_checksum(bytes);
    let complement = checksum ^ 0xFFFF;
    bytes[COMPLEMENT_OFFSET..COMPLEMENT_OFFSET + 2].copy_from_slice(&complement.to_le_bytes());
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2].copy_from_slice(&checksum.to_le_bytes());
}

// -------------------------------------------------------------------------------------------------

/// Expand `rom` to `target_size` bytes (one of [`EXPANSION_TARGETS`]).
///
/// The new banks are `0xFF`-filled; the internal header's ROM size byte becomes
/// the new size's exponent (`size_in_kb = 2^byte`), and the checksum/complement
/// pair is recomputed. The returned [`Rom`] re-parses its own header as a
/// sanity check, so a corrupt result fails here instead of downstream.
pub fn expand_rom(rom: &Rom, target_size: usize) -> Result<Rom, ExpansionError> {
    let current = rom.bytes().len();
    if !matches!(current, 0x8_0000 | 0x10_0000 | 0x20_0000) {
        return Err(ExpansionError::UnsupportedSize(current));
    }
    if !EXPANSION_TARGETS.contains(&target_size) {
        return Err(ExpansionError::UnsupportedTarget(target_size));
    }
    if target_size <= current {
        return Err(ExpansionError::TargetNotLarger(target_size, current));
    }

    let mut bytes = rom.bytes().to_vec();
    bytes.resize(target_size, 0xFF);

    // ROM size byte: exponent N with 2^N KB = size.
    let size_kb = target_size / 0x400;
    debug_assert!(size_kb.is_power_of_two());
    bytes[ROM_SIZE_OFFSET] = size_kb.trailing_zeros() as u8;

    // Recompute checksum over the final image (header fields zeroed while
    // summing, per `compute_checksum`'s convention).
    rewrite_checksum(&mut bytes);

    let expanded = Rom(Arc::from(bytes.into_boxed_slice()));
    // Sanity: the expanded image must still locate its own internal header.
    crate::internal_header::RomInternalHeader::parse(&expanded)?;
    Ok(expanded)
}

/// Split raw file bytes into the SMC header (if present) and the headerless
/// image, mirroring [`Rom::new`]'s detection.
pub fn split_smc_header(file_bytes: &[u8]) -> (Option<&[u8]>, &[u8]) {
    if file_bytes.len() % 0x400 == SMC_HEADER_SIZE {
        (Some(&file_bytes[..SMC_HEADER_SIZE]), &file_bytes[SMC_HEADER_SIZE..])
    } else {
        (None, file_bytes)
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snes_utils::rom::Rom;

    /// Minimal synthetic LoROM: zeroed image with a valid internal header
    /// (name + complementary checksum pair), so `RomInternalHeader::parse`
    /// succeeds without needing the real ROM.
    fn synthetic_rom(size: usize, size_byte: u8) -> Rom {
        let mut bytes = vec![0xFFu8; size];
        let base = HEADER_BASE;
        bytes[base..base + 21].copy_from_slice(b"SYNTHETIC TEST ROM   ");
        bytes[base + 0x15] = 0x30; // FastLoRom
        bytes[base + 0x16] = 0x02; // ROM+RAM+SRAM
        bytes[base + 0x17] = size_byte;
        bytes[base + 0x19] = 0x01; // North America (rest of the image is 0xFF)
        let checksum = compute_checksum(&bytes);
        bytes[COMPLEMENT_OFFSET..COMPLEMENT_OFFSET + 2].copy_from_slice(&(checksum ^ 0xFFFF).to_le_bytes());
        bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2].copy_from_slice(&checksum.to_le_bytes());
        // Sanity: the fixture itself must parse.
        let rom = Rom::new(bytes).unwrap();
        crate::internal_header::RomInternalHeader::parse(&rom).unwrap();
        rom
    }

    #[test]
    fn expands_512k_to_4m_and_fixes_header() {
        let rom = synthetic_rom(0x8_0000, 0x09);
        let expanded = expand_rom(&rom, 0x40_0000).unwrap();
        let bytes = expanded.bytes();
        assert_eq!(bytes.len(), 0x40_0000);
        // New space is 0xFF fill.
        assert!(bytes[0x8_0000..].iter().all(|&b| b == 0xFF));
        // Original content preserved, except the 5 header bytes expansion rewrites
        // (ROM size byte + checksum/complement pair).
        let original = rom.bytes();
        assert_eq!(&bytes[..ROM_SIZE_OFFSET], &original[..ROM_SIZE_OFFSET]);
        assert_eq!(&bytes[ROM_SIZE_OFFSET + 1..COMPLEMENT_OFFSET], &original[ROM_SIZE_OFFSET + 1..COMPLEMENT_OFFSET]);
        assert_eq!(&bytes[CHECKSUM_OFFSET + 2..0x8_0000], &original[CHECKSUM_OFFSET + 2..0x8_0000]);
        // Header: size byte 0x09 -> 0x0C (4096 KB).
        assert_eq!(bytes[ROM_SIZE_OFFSET], 0x0C);
        // Checksum pair is complementary and self-consistent.
        let stored_cpl = u16::from_le_bytes([bytes[COMPLEMENT_OFFSET], bytes[COMPLEMENT_OFFSET + 1]]);
        let stored_csm = u16::from_le_bytes([bytes[CHECKSUM_OFFSET], bytes[CHECKSUM_OFFSET + 1]]);
        assert_eq!(stored_cpl ^ stored_csm, 0xFFFF);
        assert_eq!(compute_checksum(bytes), stored_csm);
        // Map mode untouched.
        assert_eq!(bytes[HEADER_BASE + 0x15], 0x30);
    }

    #[test]
    fn intermediate_targets_update_size_byte() {
        let rom = synthetic_rom(0x8_0000, 0x09);
        assert_eq!(expand_rom(&rom, 0x10_0000).unwrap().bytes()[ROM_SIZE_OFFSET], 0x0A);
        assert_eq!(expand_rom(&rom, 0x20_0000).unwrap().bytes()[ROM_SIZE_OFFSET], 0x0B);
    }

    #[test]
    fn expansion_targets_lists_larger_sizes_only() {
        assert_eq!(expansion_targets(0x8_0000), vec![0x10_0000, 0x20_0000, 0x40_0000]);
        assert_eq!(expansion_targets(0x20_0000), vec![0x40_0000]);
        assert_eq!(expansion_targets(0x40_0000), Vec::<usize>::new());
    }

    #[test]
    fn rejects_bad_inputs() {
        let rom = synthetic_rom(0x8_0000, 0x09);
        // Not a valid target at all.
        assert!(matches!(expand_rom(&rom, 0x8_0000), Err(ExpansionError::UnsupportedTarget(_))));
        assert!(matches!(expand_rom(&rom, 0x30_0000), Err(ExpansionError::UnsupportedTarget(_))));
        // Valid target, but not larger than the current image.
        let two_mb = synthetic_rom(0x20_0000, 0x0B);
        assert!(matches!(expand_rom(&two_mb, 0x10_0000), Err(ExpansionError::TargetNotLarger(_, _))));
        let odd = Rom::new(vec![0xFFu8; 0x90000]).unwrap();
        assert!(matches!(expand_rom(&odd, 0x40_0000), Err(ExpansionError::UnsupportedSize(_))));
        let maxed = synthetic_rom(0x40_0000, 0x0C);
        // Already at the LoROM cap: not expandable at all.
        assert!(matches!(expand_rom(&maxed, 0x40_0000), Err(ExpansionError::UnsupportedSize(_))));
    }

    #[test]
    fn new_space_is_visible_to_find_free_space() {
        use crate::freespace::find_free_space;
        let rom = synthetic_rom(0x8_0000, 0x09);
        let expanded = expand_rom(&rom, 0x10_0000).unwrap();
        // 0x9000 bytes won't fit in the 0xFF tail of a synthetic all-0xFF 512K
        // image scan starting at 0... use a distinctive probe instead: the
        // first 0xFF run at/after the old end of the image.
        let pc = find_free_space(expanded.bytes(), 0x8000, 0x8_0000, 0).unwrap();
        assert_eq!(pc, 0x8_0000);
    }

    #[test]
    fn split_smc_header_round_trips() {
        let mut file = vec![0xAAu8; SMC_HEADER_SIZE];
        file.extend(vec![0xFFu8; 0x8_0000]);
        let (hdr, body) = split_smc_header(&file);
        assert_eq!(hdr.unwrap().len(), SMC_HEADER_SIZE);
        assert_eq!(body.len(), 0x8_0000);
        let (hdr2, _) = split_smc_header(body);
        assert!(hdr2.is_none());
    }

    #[test]
    fn format_size_reads_naturally() {
        assert_eq!(format_size(0x8_0000), "512 KB");
        assert_eq!(format_size(0x40_0000), "4 MB");
    }
}
