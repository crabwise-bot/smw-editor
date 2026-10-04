//! Import palettes from Snes9x savestate files (Lunar Magic v3.40 parity:
//! "added support for importing palettes from Snes9x save state files").
//!
//! The palette import reads the live on-screen palette (CGRAM) out of the
//! savestate's `PPU` block. The format reference is Snes9x's
//! `snapshot.cpp` (`S9xFreezeToStream`, `FreezeBlock`, `UnfreezeBlock`,
//! `SnapPPU`) plus `ppu.h` (`struct SPPU`), both from
//! <https://github.com/snes9xgit/snes9x> (read 2026-10-03; the format logic
//! below mirrors them and cites the exact routines):
//!
//! - File header: `"#!s9xsnp:%04d\n"` (`SNAPSHOT_MAGIC` + `SNAPSHOT_VERSION`;
//!   `snapshot.h`). Older Snes9x (pre-1.52-era binary states) has no such
//!   header and is rejected.
//! - Named blocks after the header: an 11-byte `"XXX:%06d:"` header (3-char
//!   name, `:`-delimited decimal length), or `"XXX:------:"` with a
//!   big-endian u32 length packed into bytes 6..10 when the payload exceeds
//!   999999 bytes (`FreezeBlock`). Unknown blocks are skipped by length.
//! - The `PPU` block is the frozen `struct SPPU` serialized field-by-field
//!   (`UnfreezeStructFromCopy`): scalars big-endian at their C size,
//!   `uint16` arrays as big-endian words (`uint16_ARRAY_V` case in
//!   `FreezeStruct`/`UnfreezeStructFromCopy`). Walking `SnapPPU` in order
//!   gives the byte offset of `CGDATA` (256 × u16, the SNES CGRAM in native
//!   15-bit BGR — exactly the word format this editor's palettes use):
//!   - VMA: High 1 + Increment 1 + Address 2 + Mask1 2 +
//!     FullGraphicCount 2 + Shift 2 = 10 (`ppu.h`: bool8/uint8/uint16…)
//!   - WRAM: 4 (uint32)
//!   - BG[4]: (SCBase 2 + HOffset 2 + VOffset 2 + BGSize 1 + NameBase 2 +
//!     SCSize 2) × 4 = 44
//!   - BGMode 1 + BG3Priority 1 + CGFLIP 1 + CGFLIPRead 1 + CGADD 1 = 5
//!   - CGSavedByte: 1, debuted in snapshot version 11 (`INT_ENTRY(11, …)`)
//!   - → CGDATA at byte 63 for versions 6–10, byte 64 for versions 11–12.
//!   `CGDATA` itself debuted in version 6 (`ARRAY_ENTRY(6, CGDATA, …)`), so
//!   anything older cannot be located reliably and is rejected.
//!
//! Nothing else in the savestate is read or trusted: no WRAM, no SRAM, no
//! registers — just the palette.

use std::fmt;

/// Colors in a Snes9x savestate's CGRAM (the `CGDATA` block of the `PPU`
/// struct: 256 SNES 15-bit BGR color words).
pub const CGRAM_COLORS: usize = 256;
/// Serialized bytes of the CGRAM (256 big-endian u16 words).
pub const CGRAM_BYTES: usize = CGRAM_COLORS * 2;

/// `SNAPSHOT_MAGIC` from Snes9x's `snapshot.h`.
const MAGIC: &[u8] = b"#!s9xsnp";
/// Oldest snapshot version whose `PPU` layout we can locate `CGDATA` in
/// (`CGDATA` debuted in version 6).
const MIN_VERSION: u32 = 6;
/// `SNAPSHOT_VERSION` in Snes9x's `snapshot.h` at the time of writing; newer
/// versions are rejected rather than mis-parsed.
const MAX_VERSION: u32 = 12;
/// Byte offset of `CGDATA` inside the `PPU` block payload for snapshot
/// versions 6–10 (`CGSavedByte` not yet present).
const CGDATA_OFFSET_V6: usize = 63;
/// Same for versions 11–12 (`INT_ENTRY(11, CGSavedByte)` adds one byte).
const CGDATA_OFFSET_V11: usize = 64;

/// Errors from the Snes9x savestate CGRAM parser. Every failure is
/// reported before any palette state changes, so a bad file can never
/// half-apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snes9xStateError {
    /// Not a Snes9x savestate (no `#!s9xsnp:` header).
    BadMagic,
    /// The `#!s9xsnp:VVVV\n` header is truncated or malformed.
    BadHeader,
    /// Snapshot version outside 6..=12 — the `PPU` layout differs from what
    /// we compute, so the palette location would be a guess.
    BadVersion(u32),
    /// The file ends before a block's declared length.
    Truncated,
    /// No `PPU` block in the savestate.
    MissingPpuBlock,
    /// The `PPU` block payload is too short to contain `CGDATA`.
    ShortPpuBlock { need: usize, got: usize },
}

impl fmt::Display for Snes9xStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Snes9xStateError::BadMagic => {
                write!(f, "not a Snes9x savestate (missing `#!s9xsnp:` header)")
            }
            Snes9xStateError::BadHeader => {
                write!(f, "malformed Snes9x savestate header (expected `#!s9xsnp:VVVV\\n`)")
            }
            Snes9xStateError::BadVersion(v) => write!(
                f,
                "unsupported Snes9x savestate version {v} (palette import supports versions {MIN_VERSION}–{MAX_VERSION})"
            ),
            Snes9xStateError::Truncated => {
                write!(f, "truncated Snes9x savestate: file ends inside a block")
            }
            Snes9xStateError::MissingPpuBlock => {
                write!(f, "no PPU block in the Snes9x savestate")
            }
            Snes9xStateError::ShortPpuBlock { need, got } => {
                write!(f, "PPU block too short for the palette ({got} bytes, need {need})")
            }
        }
    }
}

impl std::error::Error for Snes9xStateError {}

/// A Snes9x savestate's live palette plus the snapshot version the
/// `CGDATA` offset was computed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snes9xCgram {
    /// The 256 SNES 15-bit BGR color words of the `PPU` block's `CGDATA`
    /// field.
    pub colors:  [u16; CGRAM_COLORS],
    /// Snapshot version from the file header.
    pub version: u32,
}

/// Byte offset of `CGDATA` within the `PPU` block payload for a snapshot
/// version — see the module docs for the field-by-field derivation from
/// Snes9x's `SnapPPU`/`ppu.h`.
fn cgdata_offset(version: u32) -> usize {
    if version >= 11 {
        CGDATA_OFFSET_V11
    } else {
        CGDATA_OFFSET_V6
    }
}

/// Read one block header (`FreezeBlock` format: 11 bytes, `"XXX:%06d:"` or
/// `"XXX:------:"` + big-endian u32). Returns the block name and its payload
/// length; `pos` must point at the header start.
fn read_block_header(bytes: &[u8], pos: usize) -> Result<([u8; 3], usize), Snes9xStateError> {
    let header = bytes.get(pos..pos + 11).ok_or(Snes9xStateError::Truncated)?;
    if header[3] != b':' || header[10] != b':' {
        return Err(Snes9xStateError::Truncated);
    }
    let name = [header[0], header[1], header[2]];
    let len = if header[4] == b'-' {
        // Over-999999 sizes pack the length big-endian into bytes 6..10
        // (`FreezeBlock`'s `else` branch); require the full `------` marker.
        if header[4..10] != *b"------" {
            return Err(Snes9xStateError::Truncated);
        }
        u32::from_be_bytes([header[6], header[7], header[8], header[9]]) as usize
    } else {
        let digits = std::str::from_utf8(&header[4..10]).map_err(|_| Snes9xStateError::Truncated)?;
        digits.parse::<usize>().map_err(|_| Snes9xStateError::Truncated)?
    };
    Ok((name, len))
}

/// Extract the savestate's live on-screen palette: the 256 SNES 15-bit BGR
/// color words of the `PPU` block's `CGDATA` field, plus the snapshot
/// version.
pub fn parse_cgram(bytes: &[u8]) -> Result<Snes9xCgram, Snes9xStateError> {
    // Header: `#!s9xsnp:%04d\n`.
    if bytes.len() < MAGIC.len() + 1 || &bytes[..MAGIC.len()] != MAGIC || bytes[MAGIC.len()] != b':' {
        return Err(Snes9xStateError::BadMagic);
    }
    let rest = &bytes[MAGIC.len() + 1..];
    let nl = rest.iter().position(|&b| b == b'\n').ok_or(Snes9xStateError::BadHeader)?;
    let version: u32 = std::str::from_utf8(&rest[..nl])
        .map_err(|_| Snes9xStateError::BadHeader)?
        .parse()
        .map_err(|_| Snes9xStateError::BadHeader)?;
    if version < MIN_VERSION || version > MAX_VERSION {
        return Err(Snes9xStateError::BadVersion(version));
    }
    let mut pos = MAGIC.len() + 1 + nl + 1;

    loop {
        let (name, len) = read_block_header(bytes, pos)?;
        let payload = pos + 11;
        let end = payload.checked_add(len).ok_or(Snes9xStateError::Truncated)?;
        if end > bytes.len() {
            return Err(Snes9xStateError::Truncated);
        }
        if name == *b"PPU" {
            let off = cgdata_offset(version);
            let need = off + CGRAM_BYTES;
            if len < need {
                return Err(Snes9xStateError::ShortPpuBlock { need, got: len });
            }
            let mut colors = [0u16; CGRAM_COLORS];
            for (i, c) in colors.iter_mut().enumerate() {
                let b = &bytes[payload + off + i * 2..payload + off + i * 2 + 2];
                // `uint16_ARRAY_V` serializes each word big-endian.
                *c = u16::from_be_bytes([b[0], b[1]]);
            }
            return Ok(Snes9xCgram { colors, version });
        }
        pos = end;
        if pos >= bytes.len() {
            break;
        }
    }
    Err(Snes9xStateError::MissingPpuBlock)
}

/// One 12-color palette-editor row out of the savestate CGRAM. `row` is the
/// CGRAM row index 0–15 (BG rows 0–7, sprite rows 8–15); each CGRAM row is
/// 16 colors and the editor uses the first 12.
pub fn cgram_row(cgram: &[u16; CGRAM_COLORS], row: usize) -> [u16; 12] {
    let base = row.min(15) * 16;
    let mut out = [0u16; 12];
    out.copy_from_slice(&cgram[base..base + 12]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but structurally honest Snes9x savestate: the real
    /// `#!s9xsnp:VVVV\n` header, a `NAM` block like `S9xFreezeToStream`
    /// writes first, then a `PPU` block with `count` pre-`CGDATA` filler
    /// bytes (version-dependent) followed by 256 big-endian CGRAM words,
    /// plus one unknown block afterwards to prove skipping works.
    fn build_synthetic_state(version: u32, cgram: &[u16; CGRAM_COLORS]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(format!("#!s9xsnp:{version:04}\n").as_bytes());
        // `NAM` block: `sprintf(buffer, "NAM:%06d:%s%c", 8, "Removed", 0)`.
        out.extend_from_slice(b"NAM:000008:");
        out.extend_from_slice(b"Removed\0");
        // PPU block: filler bytes (pattern-tagged so an off-by-N offset
        // reads garbage that can't match) + CGDATA big-endian.
        let off = cgdata_offset(version);
        let mut payload = vec![0xAAu8; off];
        for &c in cgram.iter() {
            payload.extend_from_slice(&c.to_be_bytes());
        }
        // Trailing bytes so the PPU payload is longer than just CGDATA.
        payload.extend_from_slice(&[0x55; 32]);
        out.extend_from_slice(format!("PPU:{:06}:", payload.len()).as_bytes());
        out.extend_from_slice(&payload);
        // An unknown block afterwards: the walker must skip it, not choke.
        out.extend_from_slice(b"ZZZ:000004:");
        out.extend_from_slice(b"\xDE\xAD\xBE\xEF");
        out
    }

    fn distinct_cgram() -> [u16; CGRAM_COLORS] {
        let mut c = [0u16; CGRAM_COLORS];
        for (i, v) in c.iter_mut().enumerate() {
            // Recognizable pattern: index in the low bits, row in the high.
            *v = ((i / 16) as u16) << 10 | (i % 16) as u16;
        }
        c
    }

    #[test]
    fn parse_version_12_round_trips() {
        let want = distinct_cgram();
        let state = build_synthetic_state(12, &want);
        let got = parse_cgram(&state).expect("v12 state must parse");
        assert_eq!(got.colors, want);
        assert_eq!(got.version, 12);
    }

    #[test]
    fn parse_version_11_and_6_offsets() {
        // v11+: CGDATA at byte 64; v6..=10: byte 63. Both must parse with
        // the right offset — the 0xAA filler would corrupt colors otherwise.
        for version in [6u32, 7, 10, 11] {
            let want = distinct_cgram();
            let state = build_synthetic_state(version, &want);
            let got = parse_cgram(&state).unwrap_or_else(|e| panic!("v{version} state must parse: {e}"));
            assert_eq!(got.colors, want, "version {version}");
            assert_eq!(got.version, version, "version {version}");
        }
    }

    #[test]
    fn rejects_bad_magic() {
        assert_eq!(parse_cgram(b"SNES savestate data"), Err(Snes9xStateError::BadMagic));
        assert_eq!(parse_cgram(b""), Err(Snes9xStateError::BadMagic));
        // A bare ROM is the most likely wrong file a user picks.
        assert_eq!(parse_cgram(&vec![0u8; 512]), Err(Snes9xStateError::BadMagic));
    }

    #[test]
    fn rejects_bad_header() {
        assert_eq!(parse_cgram(b"#!s9xsnp:"), Err(Snes9xStateError::BadHeader));
        assert_eq!(parse_cgram(b"#!s9xsnp:XX12\n"), Err(Snes9xStateError::BadHeader));
        assert_eq!(parse_cgram(b"#!s9xsnp:0012"), Err(Snes9xStateError::BadHeader));
    }

    #[test]
    fn rejects_unsupported_versions() {
        let state = build_synthetic_state(12, &distinct_cgram());
        let mut old = state.clone();
        old[9..13].copy_from_slice(b"0005");
        assert_eq!(parse_cgram(&old), Err(Snes9xStateError::BadVersion(5)));
        let mut future = state.clone();
        future[9..13].copy_from_slice(b"0013");
        assert_eq!(parse_cgram(&future), Err(Snes9xStateError::BadVersion(13)));
    }

    #[test]
    fn rejects_truncated_and_ppu_less_states() {
        let full = build_synthetic_state(12, &distinct_cgram());
        // Cut mid-PPU-payload.
        assert_eq!(parse_cgram(&full[..full.len() - 300]), Err(Snes9xStateError::Truncated));
        // Cut mid-header.
        assert_eq!(parse_cgram(&full[..20]), Err(Snes9xStateError::Truncated));
        // NAM only, no PPU.
        let mut no_ppu = Vec::new();
        no_ppu.extend_from_slice(b"#!s9xsnp:0012\nNAM:000008:Removed\0");
        assert_eq!(parse_cgram(&no_ppu), Err(Snes9xStateError::MissingPpuBlock));
    }

    #[test]
    fn rejects_short_ppu_block() {
        let mut state = Vec::new();
        state.extend_from_slice(b"#!s9xsnp:0012\n");
        // PPU payload shorter than the CGDATA offset + 512.
        let payload = vec![0xAAu8; 100];
        state.extend_from_slice(format!("PPU:{:06}:", payload.len()).as_bytes());
        state.extend_from_slice(&payload);
        assert!(matches!(parse_cgram(&state), Err(Snes9xStateError::ShortPpuBlock { .. })));
    }

    #[test]
    fn packed_big_block_length_parses() {
        // `FreezeBlock`'s >999999 path: `"XXX:------:"` + BE u32 length.
        // The PPU block comes first so the parse succeeds before ever
        // reaching the (truncated) big block; this test proves a huge
        // declared length *after* the palette doesn't disturb the walk.
        let cgram = distinct_cgram();
        let mut state = Vec::new();
        state.extend_from_slice(b"#!s9xsnp:0012\n");
        let off = cgdata_offset(12);
        let mut payload = vec![0xAAu8; off];
        for &c in &cgram {
            payload.extend_from_slice(&c.to_be_bytes());
        }
        state.extend_from_slice(format!("PPU:{:06}:", payload.len()).as_bytes());
        state.extend_from_slice(&payload);
        let mut hdr = *b"BIG:------:";
        hdr[6..10].copy_from_slice(&1_000_007u32.to_be_bytes());
        state.extend_from_slice(&hdr);
        assert_eq!(parse_cgram(&state).unwrap().colors, cgram);
    }

    #[test]
    fn cgram_row_extracts_first_12_of_row() {
        let cgram = distinct_cgram();
        let row5 = cgram_row(&cgram, 5);
        assert_eq!(row5, cgram[80..92]);
        let sprite_row = cgram_row(&cgram, 8 + 3);
        assert_eq!(sprite_row, cgram[11 * 16..11 * 16 + 12]);
        // Out-of-range row clamps instead of panicking.
        assert_eq!(cgram_row(&cgram, 99), cgram_row(&cgram, 15));
    }
}
