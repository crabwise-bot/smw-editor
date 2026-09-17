//! IPS (International Patching System) format implementation
//! Allows creation of IPS patches for ROM distribution

use std::io::Write;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("File too large for IPS format (max 16MB)")]
    FileTooLarge,
    #[error("Invalid IPS header (expected \"PATCH\")")]
    InvalidHeader,
    #[error("Truncated IPS patch at byte offset {0}")]
    Truncated(usize),
    #[error("IPS record extends past the 16MB IPS address space")]
    RecordOutOfRange,
}

/// Creates an IPS patch that transforms source into target
///
/// IPS format is simpler than BPS but limited to 16MB files.
/// This uses a linear algorithm to encode changed regions.
pub fn create_patch(source: &[u8], target: &[u8]) -> Result<Vec<u8>, IpsError> {
    // IPS format is limited to 16MB (24-bit addressing)
    if source.len() > 0xFF_FF_FF || target.len() > 0xFF_FF_FF {
        return Err(IpsError::FileTooLarge);
    }

    let mut patch = Vec::new();

    // Write header
    patch.write_all(b"PATCH")?;

    // Find all changed regions
    let max_len = source.len().max(target.len());
    let mut offset = 0;

    while offset < max_len {
        // Check if bytes differ at this offset
        let source_byte = source.get(offset);
        let target_byte = target.get(offset);

        if source_byte != target_byte {
            // Found a change, collect the changed region
            let region_start = offset;
            let mut region_data = Vec::new();

            // Collect consecutive changed bytes
            while offset < max_len {
                let src = source.get(offset);
                let tgt = target.get(offset);

                if src != tgt {
                    region_data.push(tgt.copied().unwrap_or(0));
                    offset += 1;
                } else {
                    break;
                }
            }

            // Encode this region
            // Check if it's a good candidate for RLE
            if region_data.len() >= 4 && is_rle_candidate(&region_data) {
                encode_rle_chunk(&mut patch, region_start, &region_data)?;
            } else {
                encode_raw_chunk(&mut patch, region_start, &region_data)?;
            }
        } else {
            offset += 1;
        }
    }

    // Write EOF marker
    patch.write_all(&[0x45, 0x4F, 0x46])?;

    // Write truncate size (final output size)
    write_24bit(&mut patch, target.len() as u32)?;

    Ok(patch)
}

/// Check if data would benefit from RLE encoding.
/// Uses the same most-common-byte logic as `encode_rle_chunk` so the
/// candidate check and the encoder always agree on which byte is encoded.
fn is_rle_candidate(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    let most_common = find_most_common_byte(data);
    data.iter().filter(|&&b| b == most_common).count() >= data.len() / 2
}

/// Encode a region as RLE if beneficial
fn encode_rle_chunk(patch: &mut Vec<u8>, offset: usize, data: &[u8]) -> Result<(), IpsError> {
    // For RLE: encode as a run of the most common byte
    let most_common = find_most_common_byte(data);

    // Write offset (24-bit)
    write_24bit(patch, offset as u32)?;

    // Write size as 0 (indicates RLE)
    patch.write_all(&[0, 0])?;

    // Write RLE count (16-bit)
    patch.write_all(&[(data.len() >> 8) as u8, (data.len() & 0xFF) as u8])?;

    // Write the repeated byte
    patch.write_all(&[most_common])?;

    Ok(())
}

/// Encode a region as raw data
fn encode_raw_chunk(patch: &mut Vec<u8>, offset: usize, data: &[u8]) -> Result<(), IpsError> {
    // Write offset (24-bit)
    write_24bit(patch, offset as u32)?;

    // Write size (16-bit, non-zero for raw data)
    let size = data.len();
    patch.write_all(&[(size >> 8) as u8, (size & 0xFF) as u8])?;

    // Write raw data
    patch.write_all(data)?;

    Ok(())
}

/// Find the most common byte in a slice
fn find_most_common_byte(data: &[u8]) -> u8 {
    let mut counts = [0usize; 256];
    for &byte in data {
        counts[byte as usize] += 1;
    }
    let (byte, _) = counts.iter().enumerate().max_by_key(|(_, &count)| count).unwrap();
    byte as u8
}

/// Read a 24-bit big-endian value.
fn read_24bit(data: &[u8], pos: usize) -> Result<u32, IpsError> {
    let b = data.get(pos..pos + 3).ok_or(IpsError::Truncated(pos))?;
    Ok(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
}

/// Read a 16-bit big-endian value.
fn read_16bit(data: &[u8], pos: usize) -> Result<u16, IpsError> {
    let b = data.get(pos..pos + 2).ok_or(IpsError::Truncated(pos))?;
    Ok(((b[0] as u16) << 8) | b[1] as u16)
}

/// Apply an IPS patch to `source`, producing the patched image.
///
/// Records may extend the image (Lunar IPS behavior); a trailing 3-byte size
/// after the EOF marker truncates/extends the result, matching what
/// [`create_patch`] writes. Patches produced by other tools (Lunar IPS,
/// Floating IPS) apply the same way.
pub fn apply_patch(source: &[u8], patch: &[u8]) -> Result<Vec<u8>, IpsError> {
    if !patch.starts_with(b"PATCH") {
        return Err(IpsError::InvalidHeader);
    }

    let mut out = source.to_vec();
    let mut pos = 5usize;

    while pos < patch.len() {
        // EOF marker.
        if patch[pos..].starts_with(&[0x45, 0x4F, 0x46]) {
            pos += 3;
            break;
        }

        let offset = read_24bit(patch, pos)? as usize;
        pos += 3;
        let size = read_16bit(patch, pos)? as usize;
        pos += 2;

        if size == 0 {
            // RLE record: 16-bit run length + one byte. A run length of 0
            // means 65536, per the de-facto IPS convention.
            let mut count = read_16bit(patch, pos)? as usize;
            pos += 2;
            if count == 0 {
                count = 0x1_0000;
            }
            let byte = *patch.get(pos).ok_or(IpsError::Truncated(pos))?;
            pos += 1;

            let end = offset.checked_add(count).ok_or(IpsError::RecordOutOfRange)?;
            if end > 0xFF_FF_FF {
                return Err(IpsError::RecordOutOfRange);
            }
            if end > out.len() {
                out.resize(end, 0);
            }
            out[offset..end].fill(byte);
        } else {
            let data = patch.get(pos..pos + size).ok_or(IpsError::Truncated(pos))?;
            pos += size;

            let end = offset.checked_add(size).ok_or(IpsError::RecordOutOfRange)?;
            if end > 0xFF_FF_FF {
                return Err(IpsError::RecordOutOfRange);
            }
            if end > out.len() {
                out.resize(end, 0);
            }
            out[offset..end].copy_from_slice(data);
        }
    }

    // Optional trailing 3-byte truncation size (always written by
    // `create_patch`).
    if pos + 3 == patch.len() {
        let new_len = read_24bit(patch, pos)? as usize;
        out.resize(new_len, 0);
    } else if pos != patch.len() {
        return Err(IpsError::Truncated(pos));
    }

    Ok(out)
}

/// Write a 24-bit value in big-endian format
fn write_24bit(writer: &mut impl Write, value: u32) -> std::io::Result<()> {
    writer.write_all(&[(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_patch() {
        let source = b"Hello World";
        let target = b"Hello Rust!";

        let patch = create_patch(source, target).expect("patch creation failed");

        // Should have PATCH header, some chunks, EOF marker, and truncate size
        assert!(patch.starts_with(b"PATCH"));
        assert!(patch.windows(3).any(|w| w == [0x45, 0x4F, 0x46])); // EOF
        assert!(patch.len() > 8);
    }

    #[test]
    fn test_identical_files() {
        let data = b"Same content";
        let patch = create_patch(data, data).expect("patch creation failed");

        // Should still have header, EOF, and truncate
        assert!(patch.starts_with(b"PATCH"));
        assert!(patch.windows(3).any(|w| w == [0x45, 0x4F, 0x46]));
    }

    #[test]
    fn test_single_byte_change() {
        let source = b"test";
        let target = b"best";

        let patch = create_patch(source, target).expect("patch creation failed");
        assert!(patch.starts_with(b"PATCH"));
    }

    #[test]
    fn test_file_too_large() {
        let large_data = vec![0u8; 0x100_0000]; // 16MB + 1
        let result = create_patch(&large_data, &large_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_max_size_allowed() {
        let max_data = vec![0u8; 0xFF_FF_FF]; // Exactly 16MB - 1
        let patch = create_patch(&max_data, &max_data).expect("should allow 16MB");
        assert!(patch.starts_with(b"PATCH"));
    }

    #[test]
    fn test_rle_optimization() {
        // Create data with a lot of repeated bytes
        let source = vec![0u8; 100];
        let mut target = vec![0u8; 100];
        target[50..].fill(0xFF); // Second half changed to 0xFF

        let patch = create_patch(&source, &target).expect("patch creation failed");
        assert!(patch.starts_with(b"PATCH"));
        // Should be relatively small due to RLE
        assert!(patch.len() < 100);
    }

    #[test]
    fn test_apply_round_trip() {
        let source = b"Hello World";
        let target = b"Hello Rust!";

        let patch = create_patch(source, target).expect("patch creation failed");
        let applied = apply_patch(source, &patch).expect("apply failed");
        assert_eq!(applied, target);
    }

    #[test]
    fn test_apply_rle_round_trip() {
        let source = vec![0u8; 200];
        let mut target = vec![0u8; 200];
        target[50..150].fill(0xAB);

        let patch = create_patch(&source, &target).expect("patch creation failed");
        let applied = apply_patch(&source, &patch).expect("apply failed");
        assert_eq!(applied, target);
    }

    #[test]
    fn test_apply_identical_is_noop() {
        let data = b"Same content";
        let patch = create_patch(data, data).expect("patch creation failed");
        let applied = apply_patch(data, &patch).expect("apply failed");
        assert_eq!(applied, data);
    }

    #[test]
    fn test_apply_extends_and_truncates() {
        // Target longer than source: patch extends.
        let source = vec![1u8; 10];
        let target = vec![2u8; 20];
        let patch = create_patch(&source, &target).expect("patch creation failed");
        let applied = apply_patch(&source, &patch).expect("apply failed");
        assert_eq!(applied, target);

        // Target shorter than source: trailing size truncates.
        let target2 = vec![3u8; 5];
        let patch2 = create_patch(&source, &target2).expect("patch creation failed");
        let applied2 = apply_patch(&source, &patch2).expect("apply failed");
        assert_eq!(applied2, target2);
    }

    #[test]
    fn test_apply_foreign_tool_patch() {
        // Hand-crafted patch in the exact layout Lunar IPS / Floating IPS
        // write: one raw record + EOF, no trailing size.
        let mut patch = Vec::new();
        patch.extend_from_slice(b"PATCH");
        patch.extend_from_slice(&[0x00, 0x00, 0x04]); // offset 4
        patch.extend_from_slice(&[0x00, 0x02]); // size 2
        patch.extend_from_slice(b"hi");
        patch.extend_from_slice(&[0x45, 0x4F, 0x46]); // EOF

        let applied = apply_patch(b"abcdefgh", &patch).expect("apply failed");
        assert_eq!(applied, b"abcdhigh");
    }

    #[test]
    fn test_apply_rle_run_of_zero_means_65536() {
        // RLE record with a zero count encodes a 65536-byte run.
        let mut patch = Vec::new();
        patch.extend_from_slice(b"PATCH");
        patch.extend_from_slice(&[0x00, 0x00, 0x00]); // offset 0
        patch.extend_from_slice(&[0x00, 0x00]); // RLE marker
        patch.extend_from_slice(&[0x00, 0x00]); // count 0 -> 65536
        patch.push(0x77);
        patch.extend_from_slice(&[0x45, 0x4F, 0x46]); // EOF

        let applied = apply_patch(b"", &patch).expect("apply failed");
        assert_eq!(applied.len(), 0x1_0000);
        assert!(applied.iter().all(|&b| b == 0x77));
    }

    #[test]
    fn test_apply_rejects_bad_header() {
        let err = apply_patch(b"data", b"NOT A PATCH").unwrap_err();
        assert!(matches!(err, IpsError::InvalidHeader));
    }

    #[test]
    fn test_apply_rejects_truncated() {
        let mut patch = Vec::new();
        patch.extend_from_slice(b"PATCH");
        patch.extend_from_slice(&[0x00, 0x00, 0x04]); // offset, then nothing
        assert!(matches!(apply_patch(b"data", &patch).unwrap_err(), IpsError::Truncated(_)));
    }

    #[test]
    fn test_apply_rejects_trailing_garbage() {
        let mut patch = Vec::new();
        patch.extend_from_slice(b"PATCH");
        patch.extend_from_slice(&[0x45, 0x4F, 0x46]); // EOF
        patch.push(0x99); // one stray byte, not a 3-byte size
        assert!(matches!(apply_patch(b"data", &patch).unwrap_err(), IpsError::Truncated(_)));
    }
}
