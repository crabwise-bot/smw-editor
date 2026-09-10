//! Byte→character font map for message-box (dialog) text, derived empirically.
//!
//! The game's message bytes are NOT ASCII: each byte (0x00-0x7F) is a tile
//! index into the message font tileset, drawn via SMW's "dynamic stripe
//! image" (Layer 3) mechanism (see `message_boxes`; `AND #$7F` in
//! `CODE_05B208` strips bit 7, the hold/repeat flag, before use). This module
//! derives the byte→character mapping by aligning known message byte sequences
//! against their known English texts — the same technique that powers the
//! WYSIWYG message preview.
//!
//! # Synthetic fixtures
//!
//! The unit tests below use INVENTED byte→character pairings. They are NOT the
//! real SMW font; they exist to prove the derivation algorithm handles
//! alignment, repeated bytes, the bit-7 hold/repeat flag, and control codes.
//! The true map can only be derived from a real ROM: run the ignored
//! `real_rom_dump_font_map_input` test in `message_boxes` to dump the real
//! byte sequences, pair each with its known vanilla English text, and feed the
//! pairs to [`derive_font_map`].

/// A byte (0x00-0x7F, bit 7 masked) → character mapping for message text.
#[derive(Debug, Clone)]
pub struct FontMap {
    map: [Option<char>; 128],
}

impl FontMap {
    /// Look up the character for a raw message byte. Bit 7 is the game's
    /// hold/repeat flag and is masked off, matching `AND #$7F` in
    /// `CODE_05B208`.
    pub fn char_for(&self, byte: u8) -> Option<char> {
        self.map[(byte & 0x7F) as usize]
    }

    /// Decode raw message bytes to readable text. Control-code bytes are
    /// skipped; bytes with no mapping decode as `'?'`.
    pub fn to_text(&self, bytes: &[u8], control_codes: &[u8]) -> String {
        let mut out = String::new();
        for &b in bytes {
            let b7 = b & 0x7F;
            if control_codes.contains(&b7) {
                continue;
            }
            out.push(self.map[b7 as usize].unwrap_or('?'));
        }
        out
    }
}

/// Derive a [`FontMap`] from `(byte sequence, known English text)` pairs.
///
/// `control_codes` lists the byte values (after bit-7 masking) that do not
/// produce a character — e.g. line break, end-of-message. Each consumes a byte
/// without consuming a character of text.
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

    /// "WELCOME,MARIO." — exercises a mid-message control code (line break),
    /// a trailing end-of-message control code, and a bit-7 hold/repeat byte
    /// (0x8E must map exactly like 0x0E = 'O').
    fn syn_welcome() -> Vec<u8> {
        vec![
            0x16, 0x04, 0x0B, 0x02, 0x0E, 0x0C, 0x04, 0x1D, // W E L C O M E ,
            SYN_LINE, // line break: consumes a byte, no character
            0x0C, 0x00, 0x11, 0x08, 0x8E, // M A R I O (0x8E = hold/repeat 'O')
            0x1C, // .
            SYN_END,
        ]
    }

    #[test]
    fn derivation_aligns_and_maps_consistently_across_messages() {
        let map = derive_font_map(
            &[(&syn_hello(), "HELLO WORLD!"), (&syn_welcome(), "WELCOME,MARIO.")],
            SYN_CONTROLS,
        )
        .unwrap();
        // Byte 0x0E appears in both messages and must map to 'O' in both.
        assert_eq!(map.char_for(0x0E), Some('O'));
        assert_eq!(map.char_for(0x07), Some('H'));
        assert_eq!(map.char_for(0x1A), Some(' '));
        // Full decode round-trips the known texts (control codes skipped).
        assert_eq!(map.to_text(&syn_hello(), SYN_CONTROLS), "HELLO WORLD!");
        assert_eq!(map.to_text(&syn_welcome(), SYN_CONTROLS), "WELCOME,MARIO.");
    }

    #[test]
    fn repeat_flag_bit7_masks_to_the_same_character() {
        let map = derive_font_map(&[(&syn_welcome(), "WELCOME,MARIO.")], SYN_CONTROLS).unwrap();
        assert_eq!(map.char_for(0x8E), Some('O'));
        assert_eq!(map.char_for(0x0E), Some('O'));
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
