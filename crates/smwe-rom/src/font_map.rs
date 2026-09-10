//! Byte→character font map for message-box (dialog) text, derived empirically
//! from the real SMW (U) ROM.
//!
//! The game's message bytes are NOT ASCII: each byte (0x00-0x7F) is a tile
//! index into the message font tileset, drawn via SMW's "dynamic stripe
//! image" (Layer 3) mechanism. The routine `CODE_05B208` (bank_05.asm) does:
//! 1. Load source byte into `_3`.
//! 2. Emit `byte & 0x7F` as the tile index.
//! 3. On the NEXT output cell, if `_3` had bit 7 set, emit tile `$1F` (blank)
//!    WITHOUT consuming another source byte.
//!
//! So bit 7 means "insert one blank cell after this character" — NOT
//! hold/repeat. Each message renders as 8 rows × 18 cells = 144 cells total.
//! Short messages (e.g. Ghost House, 90 source bytes) leave trailing rows
//! blank; the routine always emits 8 rows.
//!
//! # Real font map (SMW U, verified 2026-09-10)
//!
//! Derived by running all 22 vanilla messages through the real `CODE_05B1BC`
//! via `smwe_emu::emu::render_message` and aligning the 8×18 tile output
//! against the known English text:
//! - `0x00-0x19` → `A-Z` (uppercase)
//! - `0x40-0x59` → `a-z` (lowercase)
//! - `0x1A` → `!`, `0x1B` → `.`, `0x1D` → `,`, `0x1E` → `?`, `0x1F` → space
//! - `0x1C` → `"` (decorative quote around titles like "POINT OF ADVICE")
//! - `0x5D` → `'` (apostrophe)
//!
//! # Synthetic fixtures
//!
//! The unit tests below use INVENTED byte→character pairings. They are NOT the
//! real SMW font; they exist to prove the derivation algorithm handles
//! alignment, repeated bytes, the bit-7 blank-insertion, and control codes.
//! Use [`real_font_map`] for the true SMW (U) mapping.

/// A byte (0x00-0x7F, bit 7 masked) → character mapping for message text.
#[derive(Debug, Clone)]
pub struct FontMap {
    map: [Option<char>; 128],
}

impl FontMap {
    /// The real SMW (U) message font map, derived empirically from the ROM
    /// (verified 2026-09-10 by running all 22 messages through the real
    /// `CODE_05B1BC`). See module docs for the derivation method.
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
    /// blank-insertion flag and is masked off, matching `AND #$7F` in
    /// `CODE_05B208`.
    pub fn char_for(&self, byte: u8) -> Option<char> {
        self.map[(byte & 0x7F) as usize]
    }

    /// Decode raw message bytes to readable text, following the real
    /// `CODE_05B208` semantics: each byte emits its character (bit 7 masked);
    /// if bit 7 was set, a blank (`$1F`, rendered here as space) is inserted
    /// AFTER the character without consuming another byte. Control-code bytes
    /// are skipped; bytes with no mapping decode as `'?'`.
    pub fn to_text(&self, bytes: &[u8], control_codes: &[u8]) -> String {
        let mut out = String::new();
        for &b in bytes {
            let b7 = b & 0x7F;
            if control_codes.contains(&b7) {
                continue;
            }
            out.push(self.map[b7 as usize].unwrap_or('?'));
            // Bit 7: insert one blank cell after (CODE_05B208 BMI branch).
            if b & 0x80 != 0 {
                out.push(' ');
            }
        }
        out
    }

    /// Decode to 8 rows × 18 cells, matching the game's stripe output format.
    /// Short messages are padded with blanks (spaces) to fill 8 rows.
    pub fn to_rows(&self, bytes: &[u8], control_codes: &[u8]) -> [String; 8] {
        let text = self.to_text(bytes, control_codes);
        let mut rows: [String; 8] = Default::default();
        // Fill 144 cells (8×18), padding with spaces.
        let mut chars = text.chars().chain(std::iter::repeat(' '));
        for row in rows.iter_mut() {
            *row = chars.by_ref().take(18).collect();
        }
        rows
    }
}

/// Derive a [`FontMap`] from `(byte sequence, known English text)` pairs.
///
/// `control_codes` lists the byte values (after bit-7 masking) that do not
/// produce a character — e.g. line break, end-of-message. Each consumes a byte
/// without consuming a character of text.
///
/// Bit-7 handling (matching `CODE_05B208`): a byte with bit 7 set emits its
/// character AND inserts a blank cell after. In the `text`, this corresponds
/// to TWO characters: the letter followed by a space. The space is verified
/// but does not create a mapping (it's the blank `$1F`, not a font glyph).
///
/// Errors if a byte maps to two different characters, if a text runs out of
/// characters before its bytes do, or if bytes run out before the text does.
/// Any of those means a wrong pairing or a misidentified control code, never a
/// silent wrong map.
pub fn derive_font_map(pairs: &[(&[u8], &str)], control_codes: &[u8]) -> anyhow::Result<FontMap> {
    let mut map: [Option<char>; 128] = [None; 128];
    for (msg_i, (bytes, text)) in pairs.iter().enumerate() {
        let chars: Vec<char> = text.chars().collect();
        let mut ci = 0;
        for &b in bytes.iter() {
            let b7 = b & 0x7F;
            if control_codes.contains(&b7) {
                continue;
            }
            let c = chars.get(ci).copied().ok_or_else(|| {
                anyhow::anyhow!(
                    "message {msg_i}: ran out of text characters at byte {b:#04X} \
                     (missing control code, or wrong pairing?)"
                )
            })?;
            match map[b7 as usize] {
                None => map[b7 as usize] = Some(c),
                Some(prev) if prev == c => {}
                Some(prev) => anyhow::bail!(
                    "message {msg_i}: byte {b7:#04X} maps to both {prev:?} and {c:?} \
                     (wrong pairing, or a control code misidentified as text?)"
                ),
            }
            ci += 1;
            // Bit 7: the next text character must be the inserted blank (space).
            if b & 0x80 != 0 {
                let blank = chars.get(ci).copied().ok_or_else(|| {
                    anyhow::anyhow!(
                        "message {msg_i}: bit-7 byte {b:#04X} expects a trailing \
                         blank in text, but text ran out"
                    )
                })?;
                if blank != ' ' {
                    anyhow::bail!(
                        "message {msg_i}: bit-7 byte {b:#04X} expects a space \
                         after its character in text, found {blank:?} \
                         (the blank is the $1F tile, not a font glyph)"
                    );
                }
                ci += 1;
            }
        }
        if ci != chars.len() {
            anyhow::bail!(
                "message {msg_i}: {} text character(s) left unconsumed \
                 (missing bytes, or misidentified control codes?)",
                chars.len() - ci
            );
        }
    }
    Ok(FontMap { map })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic "font" used ONLY by these tests — invented pairings, not the
    // real SMW font: 0x00-0x19 = A-Z, 0x1A = space, 0x1B = '!', 0x1C = '.',
    // 0x1D = ',', 0x7E = line break (control), 0x7F = end-of-message (control).
    const SYN_LINE: u8 = 0x7E;
    const SYN_END: u8 = 0x7F;
    const SYN_CONTROLS: &[u8] = &[SYN_LINE, SYN_END];

    /// "HELLO WORLD!" — exercises plain alignment, no control codes.
    fn syn_hello() -> Vec<u8> {
        vec![
            0x07, 0x04, 0x0B, 0x0B, 0x0E, // H E L L O
            0x1A, // space
            0x16, 0x0E, 0x11, 0x0B, 0x03, // W O R L D
            0x1B, // !
            SYN_END,
        ]
    }

    /// "WELCOME,MARIO . " — exercises a mid-message control code (line break),
    /// a trailing end-of-message control code, and a bit-7 blank-insertion
    /// byte (0x8E emits 'O' then a blank; text has "O " with the space).
    fn syn_welcome() -> Vec<u8> {
        vec![
            0x16, 0x04, 0x0B, 0x02, 0x0E, 0x0C, 0x04, 0x1D, // W E L C O M E ,
            SYN_LINE, // line break: consumes a byte, no character
            0x0C, 0x00, 0x11, 0x08, 0x8E, // M A R I O+blank (0x8E = 'O'+blank)
            0x1A, // space (separate byte, not from bit-7)
            0x1C, // .
            SYN_END,
        ]
    }

    #[test]
    fn derivation_aligns_and_maps_consistently_across_messages() {
        let map = derive_font_map(
            &[(&syn_hello(), "HELLO WORLD!"), (&syn_welcome(), "WELCOME,MARIO  .")],
            SYN_CONTROLS,
        )
        .unwrap();
        // Byte 0x0E appears in both messages and must map to 'O' in both.
        assert_eq!(map.char_for(0x0E), Some('O'));
        assert_eq!(map.char_for(0x07), Some('H'));
        assert_eq!(map.char_for(0x1A), Some(' '));
        // Full decode round-trips the known texts (control codes skipped,
        // bit-7 blank inserted as space).
        assert_eq!(map.to_text(&syn_hello(), SYN_CONTROLS), "HELLO WORLD!");
        assert_eq!(map.to_text(&syn_welcome(), SYN_CONTROLS), "WELCOME,MARIO  .");
    }

    #[test]
    fn bit7_inserts_blank_after_character() {
        // 0x8E = 'O' with bit 7 set: emits 'O', then a blank (space).
        let map = derive_font_map(&[(&syn_welcome(), "WELCOME,MARIO  .")], SYN_CONTROLS).unwrap();
        assert_eq!(map.char_for(0x8E), Some('O'));
        assert_eq!(map.char_for(0x0E), Some('O'));
        assert_eq!(map.to_text(&[0x8E], SYN_CONTROLS), "O ");
    }

    #[test]
    fn unmapped_bytes_decode_as_question_mark() {
        let map = derive_font_map(&[(&syn_hello(), "HELLO WORLD!")], SYN_CONTROLS).unwrap();
        assert_eq!(map.char_for(0x10), None);
        assert_eq!(map.to_text(&[0x07, 0x10], SYN_CONTROLS), "H?");
    }

    #[test]
    fn conflicting_byte_mapping_is_an_error() {
        // Byte 0x00 maps to 'A' in the first pair but 'X' in the second.
        let err = derive_font_map(&[(&[0x00, 0x01], "AB"), (&[0x00], "X")], SYN_CONTROLS).unwrap_err();
        assert!(err.to_string().contains("maps to both"), "unexpected error: {err}");
    }

    #[test]
    fn text_longer_than_bytes_is_an_error() {
        // The byte sequence runs out while 'B' is still unconsumed.
        let err = derive_font_map(&[(&[0x00], "AB")], SYN_CONTROLS).unwrap_err();
        assert!(err.to_string().contains("unconsumed"), "unexpected error: {err}");
    }

    #[test]
    fn bytes_longer_than_text_is_an_error() {
        // The extra non-control byte 0x01 has no character left to consume.
        let err = derive_font_map(&[(&[0x00, 0x01], "A")], SYN_CONTROLS).unwrap_err();
        assert!(err.to_string().contains("0x01"), "unexpected error: {err}");
    }

    #[test]
    fn empty_input_yields_an_empty_map() {
        let map = derive_font_map(&[], &[]).unwrap();
        assert_eq!(map.char_for(0x00), None);
        assert_eq!(map.to_text(&[0x00], &[]), "?");
    }
}
