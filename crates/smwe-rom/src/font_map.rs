//! Byte→character font map for message-box (dialog) text, derived empirically
//! from the real SMW (U) ROM.
//!
//! The game's message bytes are NOT ASCII: each byte (0x00-0x7F) is a tile
//! index into the message font tileset (GFX2A, "Message Box Letters"),
//! drawn via SMW's "dynamic stripe image" (Layer 3) mechanism. The routine
//! `CODE_05B208` (bank_05.asm, U version) emits 8 rows × 18 cells:
//!
//! - At the start of each 18-cell row, the fill flag (`_3`) is cleared.
//! - For each cell: if the fill flag is set, emit tile `$1F` (blank) WITHOUT
//!   consuming a source byte.
//! - Otherwise read the next source byte into `_3`, emit `byte & 0x7F` as the
//!   tile index, and consume the byte. If the byte had bit 7 set, the fill
//!   flag remains set for the rest of the row.
//!
//! So bit 7 means: **emit this glyph, then fill the REMAINDER of the current
//! 18-cell row with `$1F` blanks**. Source consumption resumes at the start
//! of the next row. It is NOT hold/repeat, and it is NOT a single blank.
//! Every vanilla message contains exactly 8 bit-7 bytes — one row terminator
//! per row — which is why short messages (e.g. Ghost House, 90 source bytes)
//! show blank trailing rows: the routine always emits all 8 rows.
//!
//! There are no control codes: the real routine consumes every source byte
//! (while the fill flag is clear) and always emits exactly 144 cells.
//!
//! # Real font map (SMW U, verified 2026-09-10)
//!
//! Derived by running all 22 vanilla messages through the real `CODE_05B1BC`
//! via `smwe_emu::emu::render_message` and aligning the 8×18 tile output
//! against the known English text, then confirmed pixel-for-pixel against
//! GFX2A ("Message Box Letters", SNES $0BCB7B, 2bpp, 128 tiles):
//! - `0x00-0x19` → `A-Z` (uppercase)
//! - `0x40-0x59` → `a-z` (lowercase)
//! - `0x1A` → `!`, `0x1B` → `.`, `0x1D` → `,`, `0x1E` → `?`, `0x1F` → space
//! - `0x1C` → `"` (decorative quote around titles like "POINT OF ADVICE")
//! - `0x5D` → `'` (apostrophe)
//! - `0x60-0x63`, `0x64`, `0x6B`, … → non-text graphic tiles (Yoshi's
//!   signature, bonus-star icons, …), left unmapped.
//!
//! # Synthetic fixtures
//!
//! The unit tests below use INVENTED byte→character pairings. They are NOT the
//! real SMW font; they exist to prove the row-fill decoder and the derivation
//! algorithm handle alignment, repeated bytes, and the bit-7 row fill. Use
//! [`FontMap::real`] for the true SMW (U) mapping.

/// A byte (0x00-0x7F, bit 7 masked) → character mapping for message text.
#[derive(Debug, Clone)]
pub struct FontMap {
    map: [Option<char>; 128],
}

/// The 8×18 tile indices of a message, exactly as the real `CODE_05B208`
/// emits them: each cell holds `source byte & 0x7F`; a source byte with bit 7
/// set fills the remainder of its 18-cell row with `$1F` (blank); the fill
/// flag resets at the start of each row and source consumption resumes there.
///
/// Short input is padded with `$1F` blanks (the real routine would keep
/// reading past the message into whatever follows in ROM; padding is the sane
/// editor behavior for a truncated/edited message). Bytes beyond the 8 rows
/// are ignored.
pub fn message_cells(bytes: &[u8]) -> [[u8; 18]; 8] {
    let mut grid = [[0x1Fu8; 18]; 8];
    let mut y = 0usize;
    for row in grid.iter_mut() {
        let mut fill = false;
        for cell in row.iter_mut() {
            if fill {
                *cell = 0x1F;
            } else if let Some(&b) = bytes.get(y) {
                y += 1;
                if b & 0x80 != 0 {
                    fill = true;
                }
                *cell = b & 0x7F;
            } else {
                *cell = 0x1F;
            }
        }
    }
    grid
}

impl FontMap {
    /// The real SMW (U) message font map, derived empirically from the ROM
    /// (verified 2026-09-10 by running all 22 messages through the real
    /// `CODE_05B1BC`, and confirmed against the GFX2A "Message Box Letters"
    /// tile graphics). See module docs for the derivation method.
    pub fn real() -> Self {
        let mut map: [Option<char>; 128] = [None; 128];
        // 0x00-0x19: A-Z (uppercase)
        for (i, c) in ('A'..='Z').enumerate() {
            map[i] = Some(c);
        }
        // 0x40-0x59: a-z (lowercase)
        for (i, c) in ('a'..='z').enumerate() {
            map[0x40 + i] = Some(c);
        }
        // Punctuation and space
        map[0x1A] = Some('!');
        map[0x1B] = Some('.');
        map[0x1C] = Some('"'); // decorative quote around titles
        map[0x1D] = Some(',');
        map[0x1E] = Some('?');
        map[0x1F] = Some(' ');
        map[0x5D] = Some('\''); // apostrophe
        Self { map }
    }

    /// Look up the character for a raw message byte. Bit 7 is the game's
    /// row-fill flag and is masked off, matching `AND #$7F` in `CODE_05B208`.
    pub fn char_for(&self, byte: u8) -> Option<char> {
        self.map[(byte & 0x7F) as usize]
    }

    /// Decode raw message bytes to 8 rows × 18 characters, following the real
    /// `CODE_05B208` via [`message_cells`]. Bytes with no mapping (non-text
    /// graphic tiles) decode as `'?'`.
    pub fn to_rows(&self, bytes: &[u8]) -> [String; 8] {
        message_cells(bytes).map(|row| row.iter().map(|&b| self.map[b as usize].unwrap_or('?')).collect())
    }

    /// The 8 decoded rows joined by newlines.
    pub fn to_text(&self, bytes: &[u8]) -> String {
        self.to_rows(bytes).join("\n")
    }
}

/// Derive a [`FontMap`] from `(byte sequence, expected 8×18 text rows)` pairs.
///
/// Each pair's text is the 8 rows of 18 characters the message decodes to.
/// The walk mirrors `CODE_05B208` exactly: for each row, the fill flag starts
/// clear; a bit-7 source byte maps `byte & 0x7F` to its text cell and then
/// every remaining cell of that row must be a space (the `$1F` fill); the
/// next row resumes consuming source bytes.
///
/// Errors if a byte maps to two different characters, if the source bytes run
/// out before the 8 rows do, if bytes are left unconsumed, if a row isn't 18
/// characters, or if a fill cell isn't a space. Any of those means a wrong
/// pairing, never a silent wrong map.
pub fn derive_font_map(pairs: &[(&[u8], [&str; 8])]) -> anyhow::Result<FontMap> {
    let mut map: [Option<char>; 128] = [None; 128];
    for (msg_i, (bytes, rows)) in pairs.iter().enumerate() {
        let rows: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
        for (ri, row) in rows.iter().enumerate() {
            if row.len() != 18 {
                anyhow::bail!("message {msg_i} row {ri}: expected 18 characters, found {}", row.len());
            }
        }
        let mut y = 0usize;
        for (ri, row) in rows.iter().enumerate() {
            let mut fill = false;
            for (ci, &c) in row.iter().enumerate() {
                if fill {
                    if c != ' ' {
                        anyhow::bail!(
                            "message {msg_i} row {ri} cell {ci}: bit-7 fill expects a space, found {c:?}"
                        );
                    }
                    continue;
                }
                let &b = bytes.get(y).ok_or_else(|| {
                    anyhow::anyhow!("message {msg_i}: ran out of source bytes at row {ri} cell {ci}")
                })?;
                y += 1;
                let b7 = b & 0x7F;
                match map[b7 as usize] {
                    None => map[b7 as usize] = Some(c),
                    Some(prev) if prev == c => {}
                    Some(prev) => anyhow::bail!(
                        "message {msg_i}: byte {b7:#04X} maps to both {prev:?} and {c:?}"
                    ),
                }
                if b & 0x80 != 0 {
                    fill = true;
                }
            }
        }
        if y != bytes.len() {
            anyhow::bail!("message {msg_i}: {} source byte(s) left unconsumed", bytes.len() - y);
        }
    }
    Ok(FontMap { map })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic "font" used ONLY by these tests — invented pairings, not the
    // real SMW font: 0x00='A', 0x01='B', 0x02='C', 0x1F=' ' (space).
    const PAD_ROW: &str = "                  "; // 18 spaces

    /// One byte that decodes to a full blank row: 0x9F = space + bit-7 fill.
    const BLANK: u8 = 0x9F;

    #[test]
    fn derivation_aligns_and_maps_consistently_across_messages() {
        // Row 0: 'A','B'+fill -> "AB" + 16 spaces (2 source bytes).
        // Row 1: 'C','A'+fill -> "CA" + 16 spaces (2 source bytes, exercises
        // cross-row consistency: 0x00 must map to 'A' in both rows).
        let row0 = "AB                ";
        let row1 = "CA                ";
        let bytes: Vec<u8> = vec![0x00, 0x81, 0x02, 0x80, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = [row0, row1, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let map = derive_font_map(&[(&bytes, rows)]).unwrap();
        assert_eq!(map.char_for(0x00), Some('A'));
        assert_eq!(map.char_for(0x81), Some('B')); // bit 7 masked
        assert_eq!(map.char_for(0x02), Some('C'));
        // 0x1F is mapped to space by the blank-row bytes (0x9F & 0x7F).
        assert_eq!(map.char_for(0x1F), Some(' '));
        let decoded = map.to_rows(&bytes);
        assert_eq!(decoded[0], row0);
        assert_eq!(decoded[1], row1);
        for r in &decoded[2..] {
            assert_eq!(r, PAD_ROW);
        }
    }

    #[test]
    fn bit7_fills_rest_of_row_not_just_one_blank() {
        // 0x81 = 'B' with bit 7: emits 'B', then fills the remaining 17
        // cells of row 0 with blanks. This is the corrected semantics: the
        // old (wrong) model inserted a single blank.
        let bytes: Vec<u8> = vec![0x81, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = ["B                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let map = derive_font_map(&[(&bytes, rows)]).unwrap();
        let decoded = map.to_rows(&bytes);
        assert_eq!(decoded[0], "B                 ");
        assert_eq!(decoded[0].len(), 18);
        for r in &decoded[1..] {
            assert_eq!(r, PAD_ROW);
        }
        // Flat text keeps the row structure.
        assert_eq!(map.to_text(&bytes).lines().next().unwrap(), "B                 ");
    }

    #[test]
    fn bit7_fill_resets_each_row_and_source_resumes() {
        // 0x81 fills the rest of ROW 0; the next source byte (0x80='A') is
        // consumed at the start of ROW 1, not in row 0. This is the key
        // behavioral difference from "insert one blank".
        let bytes: Vec<u8> = vec![0x81, 0x80, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = ["B                 ", "A                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let map = derive_font_map(&[(&bytes, rows)]).unwrap();
        let decoded = map.to_rows(&bytes);
        assert_eq!(decoded[0], "B                 ");
        assert_eq!(decoded[1], "A                 ");
        // message_cells agrees: byte 1 is NOT consumed in row 0.
        let cells = message_cells(&[0x81, 0x80]);
        assert_eq!(cells[0][0], 0x01);
        assert_eq!(cells[0][1], 0x1F); // fill, not the 0x80 byte
        assert_eq!(cells[1][0], 0x00); // consumed here
    }

    #[test]
    fn bit7_as_last_cell_fills_nothing() {
        // 18 source bytes, the last with bit 7: no cells remain to fill.
        let mut bytes: Vec<u8> = vec![0x00; 17];
        bytes.push(0x80); // 'A' + fill flag, but row is already full
        bytes.extend([BLANK; 7]);
        let rows = ["AAAAAAAAAAAAAAAAAA", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let map = derive_font_map(&[(&bytes, rows)]).unwrap();
        assert_eq!(map.to_rows(&bytes)[0], "AAAAAAAAAAAAAAAAAA");
    }

    #[test]
    fn unmapped_bytes_decode_as_question_mark() {
        let bytes: Vec<u8> = vec![0x80, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = ["A                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let map = derive_font_map(&[(&bytes, rows)]).unwrap();
        assert_eq!(map.char_for(0x02), None);
        assert_eq!(map.to_rows(&[0x02])[0].chars().next().unwrap(), '?');
    }

    #[test]
    fn short_input_is_padded_with_blanks() {
        // Fewer bytes than 8 rows: the real routine would read past the
        // message; we pad with blanks instead.
        let map = FontMap::real();
        let rows = map.to_rows(&[0x07]); // 'H'
        assert_eq!(rows[0], "H                 ");
        for r in &rows[1..] {
            assert_eq!(r, PAD_ROW);
        }
    }

    #[test]
    fn conflicting_byte_mapping_is_an_error() {
        let bytes: Vec<u8> = vec![0x80, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows_a = ["A                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let rows_x = ["X                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let err = derive_font_map(&[(&bytes, rows_a), (&bytes, rows_x)]).unwrap_err();
        assert!(err.to_string().contains("maps to both"), "unexpected error: {err}");
    }

    #[test]
    fn fill_cell_must_be_a_space() {
        // 0x81 sets fill; the expected text wrongly has 'X' in a fill cell.
        let mut row0 = String::from("B");
        row0.push('X');
        row0.push_str(&" ".repeat(16));
        let bytes: Vec<u8> = vec![0x81, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = [row0.as_str(), PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let err = derive_font_map(&[(&bytes, rows)]).unwrap_err();
        assert!(err.to_string().contains("fill expects a space"), "unexpected error: {err}");
    }

    #[test]
    fn bytes_longer_than_rows_is_an_error() {
        // 9 bytes but the 8 rows only consume 8: one left unconsumed.
        let bytes: Vec<u8> = vec![0x81, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = ["B                 ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let err = derive_font_map(&[(&bytes, rows)]).unwrap_err();
        assert!(err.to_string().contains("unconsumed"), "unexpected error: {err}");
    }

    #[test]
    fn bytes_shorter_than_rows_is_an_error() {
        // Row 0 needs 2 bytes ("AB") but only 1 is provided.
        let bytes: Vec<u8> = vec![0x00];
        let rows = ["AB                ", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let err = derive_font_map(&[(&bytes, rows)]).unwrap_err();
        assert!(err.to_string().contains("ran out of source bytes"), "unexpected error: {err}");
    }

    #[test]
    fn row_must_be_18_characters() {
        let bytes: Vec<u8> = vec![0x80, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK, BLANK];
        let rows = ["too short", PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW, PAD_ROW];
        let err = derive_font_map(&[(&bytes, rows)]).unwrap_err();
        assert!(err.to_string().contains("expected 18 characters"), "unexpected error: {err}");
    }

    #[test]
    fn empty_input_yields_an_empty_map() {
        let map = derive_font_map(&[]).unwrap();
        assert_eq!(map.char_for(0x00), None);
        assert_eq!(map.to_rows(&[0x00])[0].chars().next().unwrap(), '?');
    }
}
