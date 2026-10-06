//! GFX/ExGFX file-name parsing — Lunar Magic v3.70 "Allow Descriptive GFX
//! File Names" (General Options, on by default).
//!
//! LM accepts graphics file names of the form `GFX##T.bin` / `ExGFX###T.bin`,
//! where `T` is arbitrary descriptive text (help topic "File : Insert
//! ExGFX"). The accepted shapes are:
//!
//! ```text
//! GFX##.bin        strict vanilla GFX name (## = exactly 2 hex digits)
//! ExGFX###.bin     strict ExGFX name (### = 1-3 hex digits, 0x80-0xFFF)
//! GFX##T.bin       descriptive vanilla GFX name (T = non-empty text)
//! ExGFX###T.bin    descriptive ExGFX name
//! ```
//!
//! Prefix and extension matching are ASCII case-insensitive (Windows file
//! names). Hex digits are consumed greedily: for ExGFX the first 1-3 hex
//! digits after the prefix are the index, the rest is descriptive text;
//! for GFX exactly 2 hex digits are the index, the rest is descriptive
//! text. Descriptive text must be non-empty (after trimming) and must not
//! contain a path separator (`/` or `\`) — the parser takes a bare file
//! name, not a path.
//!
//! Index ranges mirror the rest of the crate: ExGFX indices 0x80-0xFFF
//! ([`exgfx::EXGFX_FIRST_INDEX`]..=[`exgfx::EXGFX_MAX_INDEX`]), vanilla GFX
//! indices 0x00-0x33 (the game's graphics file list tops out at `GFX33`).

use crate::exgfx::{EXGFX_FIRST_INDEX, EXGFX_MAX_INDEX};

/// Highest vanilla GFX file number (the game's GFX file list tops out at
/// `GFX33`; see `GraphicsState::file_for` in `graphics/mod.rs`).
pub const VANILLA_GFX_MAX_INDEX: u16 = 0x33;

/// Which kind of graphics file a parsed name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GfxFileKind {
    /// Vanilla graphics file `GFX##[.T].bin` (indices 0x00-0x33).
    Vanilla,
    /// Extra graphics file `ExGFX###[.T].bin` (indices 0x80-0xFFF).
    ExGfx,
}

/// A parsed GFX/ExGFX file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGfxFileName {
    /// Vanilla or ExGFX file.
    pub kind:             GfxFileKind,
    /// File index: 0x00-0x33 for vanilla, 0x80-0xFFF for ExGFX.
    pub index:            u16,
    /// Descriptive suffix `T`, or `None` for the strict `GFX##.bin` /
    /// `ExGFX###.bin` form.
    pub descriptive_text: Option<String>,
}

/// Parse a bare GFX/ExGFX file name. Returns `None` when the name does not
/// follow LM's `GFX##T.bin` / `ExGFX###T.bin` shape (wrong prefix, wrong
/// extension, non-hex or missing digits, out-of-range index, or descriptive
/// text containing a path separator).
pub fn parse_gfx_filename(name: &str) -> Option<ParsedGfxFileName> {
    let name = name.trim();
    // Strip the `.bin` extension (ASCII case-insensitive).
    let stem = name.strip_suffix(".bin").or_else(|| name.strip_suffix(".BIN")).or_else(|| {
        let lower = name.to_ascii_lowercase();
        lower.strip_suffix(".bin").map(|_| &name[..name.len() - 4])
    })?;
    if stem.is_empty() {
        return None;
    }
    // Check the longer `ExGFX` prefix first so an `ExGFX...` name is never
    // tried against the shorter `GFX` prefix.
    if let Some(rest) = stem.strip_prefix("ExGFX").or_else(|| stem.strip_prefix("exgfx")) {
        return parse_with(rest, GfxFileKind::ExGfx, 1, 3, EXGFX_FIRST_INDEX, EXGFX_MAX_INDEX);
    }
    if let Some(rest) = stem.strip_prefix("GFX").or_else(|| stem.strip_prefix("gfx")) {
        return parse_with(rest, GfxFileKind::Vanilla, 2, 2, 0, VANILLA_GFX_MAX_INDEX);
    }
    None
}

/// Parse `rest` (stem minus the kind prefix) as `<hex digits><descriptive
/// text>`: `min_hex`..=`max_hex` greedy hex digits are the index (range
/// checked against `lo`..=`hi`), the remainder (if any) is the descriptive
/// text.
fn parse_with(
    rest: &str, kind: GfxFileKind, min_hex: usize, max_hex: usize, lo: u16, hi: u16,
) -> Option<ParsedGfxFileName> {
    let mut digit_count = 0usize;
    for ch in rest.chars() {
        if ch.is_ascii_hexdigit() && digit_count < max_hex {
            digit_count += 1;
        } else {
            break;
        }
    }
    if digit_count < min_hex {
        return None;
    }
    let (hex_part, text_part) = rest.split_at(digit_count);
    let index = u16::from_str_radix(hex_part, 16).ok()?;
    if !(lo..=hi).contains(&index) {
        return None;
    }
    let text = text_part.trim();
    let descriptive_text = if text.is_empty() {
        None
    } else if text.contains(['/', '\\']) {
        // A path snuck in — reject rather than mislabel a directory name as
        // descriptive text.
        return None;
    } else {
        Some(text.to_string())
    };
    Some(ParsedGfxFileName { kind, index, descriptive_text })
}

/// Rebuild the strict (no-descriptive-text) LM file name for a parsed name,
/// e.g. `ExGFX080.bin` / `GFX0C.bin`.
pub fn strict_gfx_filename(parsed: &ParsedGfxFileName) -> String {
    match parsed.kind {
        GfxFileKind::Vanilla => format!("GFX{:02X}.bin", parsed.index),
        GfxFileKind::ExGfx => format!("ExGFX{:03X}.bin", parsed.index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_names() {
        let p = parse_gfx_filename("ExGFX80.bin").unwrap();
        assert_eq!(p, ParsedGfxFileName {
            kind:             GfxFileKind::ExGfx,
            index:            0x80,
            descriptive_text: None,
        });

        let p = parse_gfx_filename("ExGFXFFF.bin").unwrap();
        assert_eq!(p.index, 0xFFF);

        let p = parse_gfx_filename("GFX0C.bin").unwrap();
        assert_eq!(p, ParsedGfxFileName {
            kind:             GfxFileKind::Vanilla,
            index:            0x0C,
            descriptive_text: None,
        });

        // Case-insensitive prefix and extension.
        let p = parse_gfx_filename("exgfx80.bin").unwrap();
        assert_eq!(p.index, 0x80);
        let p = parse_gfx_filename("ExGFX80.BIN").unwrap();
        assert_eq!(p.index, 0x80);
        let p = parse_gfx_filename("gfx0c.Bin").unwrap();
        assert_eq!(p, ParsedGfxFileName {
            kind:             GfxFileKind::Vanilla,
            index:            0x0C,
            descriptive_text: None,
        });
    }

    #[test]
    fn descriptive_names() {
        let p = parse_gfx_filename("ExGFX80Mario tiles.bin").unwrap();
        assert_eq!(p, ParsedGfxFileName {
            kind:             GfxFileKind::ExGfx,
            index:            0x80,
            descriptive_text: Some("Mario tiles".to_string()),
        });

        // One hex digit is allowed for ExGFX (0x80-0xFFF still enforced).
        let p = parse_gfx_filename("ExGFXA9 my tiles.bin").unwrap();
        assert_eq!(p.index, 0xA9);
        assert_eq!(p.descriptive_text, Some("my tiles".to_string()));

        let p = parse_gfx_filename("GFX12forest.bin").unwrap();
        assert_eq!(p, ParsedGfxFileName {
            kind:             GfxFileKind::Vanilla,
            index:            0x12,
            descriptive_text: Some("forest".to_string()),
        });
    }

    #[test]
    fn rejections() {
        // ExGFX index below 0x80.
        assert!(parse_gfx_filename("ExGFX8.bin").is_none());
        assert!(parse_gfx_filename("ExGFX7F.bin").is_none());
        // Any 3-digit hex index is <= 0xFFF, so with greedy hex consumption
        // there is no way to write an out-of-range 3-digit ExGFX index;
        // extra digits just become descriptive text.
        let p = parse_gfx_filename("ExGFX1000.bin").unwrap();
        assert_eq!(p.index, 0x100);
        assert_eq!(p.descriptive_text, Some("0".to_string()));
        // Vanilla index above 0x33.
        assert!(parse_gfx_filename("GFX80.bin").is_none());
        assert!(parse_gfx_filename("GFX80T.bin").is_none());
        assert!(parse_gfx_filename("GFX34.bin").is_none());
        // Wrong shapes.
        assert!(parse_gfx_filename("ExGFX.bin").is_none());
        assert!(parse_gfx_filename("GFX1.bin").is_none()); // ## is exactly 2 digits
        assert!(parse_gfx_filename("GFX.bin").is_none());
        assert!(parse_gfx_filename("tiles.bin").is_none());
        assert!(parse_gfx_filename("ExGFX80.png").is_none());
        assert!(parse_gfx_filename("ExGFXZZ.bin").is_none());
        // Path separator in the descriptive text.
        assert!(parse_gfx_filename("ExGFX80a/b.bin").is_none());
        assert!(parse_gfx_filename("ExGFX80a\\b.bin").is_none());
        // Empty.
        assert!(parse_gfx_filename("").is_none());
        assert!(parse_gfx_filename(".bin").is_none());
    }

    #[test]
    fn greedy_hex_consumption_is_documented() {
        // Up to 3 hex digits belong to the ExGFX index; the rest is text.
        let p = parse_gfx_filename("ExGFX800F.bin").unwrap();
        assert_eq!(p.index, 0x800);
        assert_eq!(p.descriptive_text, Some("F".to_string()));
        // Vanilla takes exactly 2 digits.
        let p = parse_gfx_filename("GFX0CDe.bin").unwrap();
        assert_eq!(p.index, 0x0C);
        assert_eq!(p.descriptive_text, Some("De".to_string()));
    }

    #[test]
    fn strict_filename_rebuild() {
        let p = parse_gfx_filename("ExGFX80Mario tiles.bin").unwrap();
        assert_eq!(strict_gfx_filename(&p), "ExGFX080.bin");
        let p = parse_gfx_filename("GFX0C.bin").unwrap();
        assert_eq!(strict_gfx_filename(&p), "GFX0C.bin");
    }
}
