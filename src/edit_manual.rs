//! Byte-level model for Lunar Magic's **"Edit Manual"** command (v1.91).
//!
//! LM's Edit menu → "Edit Manual" (also Alt+Right-click on an object/sprite)
//! lets power users edit an object or sprite's raw data bytes by hand,
//! including the extension fields of multibyte objects/sprites (v1.80).
//!
//! This module is the pure, UI-free byte codec both the level editor's
//! manual-editing dialog and the headless screenshot binary share, so the
//! two can never drift. All layouts below are the real SMW formats
//! (big-endian byte order), matching `smwe_rom::objects::Object` and
//! `smwe_rom::level::SpriteInstance`.

/// What a 3-byte object-stream entry decodes to.
///
/// The extended-entry pattern (`b0 & 0x60 == 0 && b1 & 0xF0 == 0`) is shared
/// by extended objects, exits, and screen jumps — the third byte disambiguates
/// (this is exactly how `smwe_rom::objects::Object` classifies them).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ObjectEntryKind {
    /// `NBBYYYYY bbbbXXXX SSSSSSSS`
    Standard,
    /// `N00YYYYY 0000XXXX BBBBBBBB` with `BBBBBBBB >= 0x02`
    Extended,
    /// `000ppppp 0000w0sh 00000000 dddddddd`
    Exit,
    /// `000HHHHH 00000000 00000001`
    ScreenJump,
}

/// A decoded 3-byte object entry.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DecodedObject {
    pub kind:       ObjectEntryKind,
    /// Bit 7 of byte 0 — the "new screen" flag as stored in the stream.
    pub new_screen: bool,
    /// Standard object number (`BBbbbb`, 6 bits) or extended object number
    /// (byte 2). For exits this is the destination/secondary ID low bits;
    /// for screen jumps it is the screen number.
    pub id:         u8,
    /// Local X within the screen (`XXXX`, 4 bits).
    pub x_field:    u8,
    /// Local Y within the screen (`YYYYY`, 5 bits).
    pub y_field:    u8,
    /// Standard objects: the settings byte. Extended objects: same as `id`.
    /// Exits: byte 3 (destination ID low byte) is not part of this triple.
    pub settings:   u8,
}

/// Classify a 3-byte object entry.
pub fn classify_object_bytes(b: [u8; 3]) -> ObjectEntryKind {
    if b[0] & 0x60 == 0 && b[1] & 0xF0 == 0 {
        if b[2] == 0 {
            ObjectEntryKind::Exit
        } else if b[2] == 1 {
            ObjectEntryKind::ScreenJump
        } else {
            ObjectEntryKind::Extended
        }
    } else {
        ObjectEntryKind::Standard
    }
}

/// Decode a 3-byte object entry into its fields.
pub fn decode_object_bytes(b: [u8; 3]) -> DecodedObject {
    let kind = classify_object_bytes(b);
    let new_screen = b[0] & 0x80 != 0;
    let (id, x_field, y_field, settings) = match kind {
        ObjectEntryKind::Standard => {
            ((((b[0] >> 5) & 0x03) << 4) | ((b[1] >> 4) & 0x0F), b[1] & 0x0F, b[0] & 0x1F, b[2])
        }
        ObjectEntryKind::Extended => (b[2], b[1] & 0x0F, b[0] & 0x1F, b[2]),
        // Exit / ScreenJump: decode the fields that exist in the triple so
        // the dialog can show *why* the bytes were refused.
        ObjectEntryKind::Exit => (b[0] & 0x1F, b[1] & 0x0F, b[0] & 0x1F, b[2]),
        ObjectEntryKind::ScreenJump => (b[0] & 0x1F, 0, 0, b[2]),
    };
    DecodedObject { kind, new_screen, id, x_field, y_field, settings }
}

/// Encode a standard or extended object's 3 stream bytes.
///
/// `id`: standard object number (`BBbbbb`) or extended object number.
/// `settings`: standard settings byte (ignored for extended objects, where it
/// doubles as the object number).
/// `x_field`/`y_field`: local screen coords (4 and 5 bits).
/// `new_screen`: the stream's new-screen flag (bit 7 of byte 0).
pub fn encode_object_bytes(
    id: u8, settings: u8, is_extended: bool, x_field: u8, y_field: u8, new_screen: bool,
) -> [u8; 3] {
    let n = u8::from(new_screen) << 7;
    if is_extended {
        [n | (y_field & 0x1F), x_field & 0x0F, id]
    } else {
        [n | ((id & 0x30) << 1) | (y_field & 0x1F), ((id & 0x0F) << 4) | (x_field & 0x0F), settings]
    }
}

/// A decoded 3-byte sprite entry.
///
/// Layout: `b0 = yyyyEEeY` (y low nibble, extra bits, y high bit),
/// `b1 = XXXXssss` (x tile, screen), `b2 = sprite ID` — matching
/// `SpriteInstance`'s `xy_pos`/`extra_bits`/`screen_number` decoders and
/// `EditableSpriteLayer::serialize_bytes`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DecodedSprite {
    /// X tile within the screen (4 bits).
    pub x_tile:     u8,
    /// Y tile (5 bits).
    pub y_tile:     u8,
    /// Screen number (5 bits).
    pub screen:     u8,
    /// Sprite extra bits (2 bits).
    pub extra_bits: u8,
    /// Sprite ID.
    pub sprite_id:  u8,
}

/// Decode a 3-byte sprite entry into its fields.
pub fn decode_sprite_bytes(b: [u8; 3]) -> DecodedSprite {
    DecodedSprite {
        x_tile:     b[1] >> 4,
        y_tile:     ((b[0] & 0x01) << 4) | (b[0] >> 4),
        screen:     ((b[0] & 0x02) << 3) | (b[1] & 0x0F),
        extra_bits: (b[0] >> 2) & 0x03,
        sprite_id:  b[2],
    }
}

/// Encode a sprite's 3 stream bytes from its fields.
pub fn encode_sprite_bytes(x_tile: u8, y_tile: u8, screen: u8, extra_bits: u8, sprite_id: u8) -> [u8; 3] {
    let b0 =
        ((y_tile & 0x0F) << 4) | (((screen >> 4) & 0x01) << 1) | ((extra_bits & 0x03) << 2) | ((y_tile >> 4) & 0x01);
    let b1 = ((x_tile & 0x0F) << 4) | (screen & 0x0F);
    [b0, b1, sprite_id]
}

/// Absolute tile coords of a sprite from its screen/local fields.
///
/// Horizontal levels: the screen is a 16-tile-wide strip (`x = screen*16 +
/// x_tile`). Vertical levels: screens tile 2-wide (`sx = screen % 2`,
/// `sy = screen / 2`; `x = sx*16 + x_tile`, `y = sy*32 + y_tile`) — the
/// inverse of `EditableSpriteLayer::from_rom_sprite_layer`.
pub fn sprite_absolute_coords(screen: u8, x_tile: u8, y_tile: u8, vertical_level: bool) -> (u32, u32) {
    if vertical_level {
        let sx = (screen % 2) as u32;
        let sy = (screen / 2) as u32;
        (sx * 16 + x_tile as u32, sy * 32 + y_tile as u32)
    } else {
        (screen as u32 * 16 + x_tile as u32, y_tile as u32)
    }
}

/// Absolute tile coords of an object from its decoded fields and screen.
///
/// Mirrors `EditableObject::from_raw`: horizontal levels add the screen's
/// 16-tile strip to X; vertical levels swap the axes (the `YYYYY` field is
/// the absolute X, the `XXXX` field offsets Y within the screen).
pub fn object_absolute_coords(d: &DecodedObject, screen: u32, vertical_level: bool) -> (u32, u32) {
    if vertical_level {
        (d.y_field as u32, screen * 16 + d.x_field as u32)
    } else {
        (screen * 16 + d.x_field as u32, d.y_field as u32)
    }
}

/// Parse one hex byte from a dialog text field.
///
/// Accepts 1–2 hex digits with an optional `$` or `0x` prefix, like LM's
/// hex fields.
pub fn parse_hex_byte(s: &str) -> Result<u8, String> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X").trim_start_matches('$');
    if t.is_empty() {
        return Err("empty byte".to_string());
    }
    if t.len() > 2 {
        return Err(format!("\"{s}\": expected 1–2 hex digits"));
    }
    u8::from_str_radix(t, 16).map_err(|_| format!("\"{s}\": not valid hex"))
}

/// Human-readable decoded summary of 3 object stream bytes, exactly as shown
/// in the Edit Manual dialog. `is_extended` is the entry kind being edited:
/// bytes decoding to the other kind — or to an exit/screen-jump pattern —
/// are refused, since those aren't editable objects.
pub fn object_decoded_summary(bytes: [u8; 3], is_extended: bool) -> Result<String, String> {
    let d = decode_object_bytes(bytes);
    match d.kind {
        ObjectEntryKind::Exit => {
            return Err("These bytes decode as a level exit — Edit Manual only edits objects; \
                        use the Secondary Entrances editor for exits"
                .to_string());
        }
        ObjectEntryKind::ScreenJump => {
            return Err("These bytes decode as a screen jump, not an object".to_string());
        }
        ObjectEntryKind::Standard if is_extended => {
            return Err("These bytes decode as a standard object, but the selected entry is an \
                        extended object — keep the same entry kind"
                .to_string());
        }
        ObjectEntryKind::Extended if !is_extended => {
            return Err("These bytes decode as an extended object, but the selected entry is a \
                        standard object — keep the same entry kind"
                .to_string());
        }
        _ => {}
    }
    if d.kind == ObjectEntryKind::Standard {
        Ok(format!(
            "Standard object 0x{:02X} · X=0x{:X} Y=0x{:02X} · settings 0x{:02X}{}",
            d.id,
            d.x_field,
            d.y_field,
            d.settings,
            if d.new_screen { " · new-screen flag" } else { "" }
        ))
    } else {
        Ok(format!("Extended object 0x{:02X} · X=0x{:X} Y=0x{:02X}", d.id, d.x_field, d.y_field))
    }
}

/// Human-readable decoded summary of 3 sprite stream bytes, exactly as shown
/// in the Edit Manual dialog. Every 3-byte pattern decodes as a sprite.
pub fn sprite_decoded_summary(bytes: [u8; 3]) -> String {
    let d = decode_sprite_bytes(bytes);
    format!(
        "Sprite 0x{:02X} · tile ({}, {}) · screen 0x{:02X} · extra bits {}",
        d.sprite_id, d.x_tile, d.y_tile, d.screen, d.extra_bits
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_object_round_trip() {
        // Standard object: id 0x2B (BB=10, bbbb=1011), x=0xC, y=0x1A,
        // settings 0x53, new screen set.
        let bytes = encode_object_bytes(0x2B, 0x53, false, 0xC, 0x1A, true);
        assert_eq!(bytes, [0x80 | (0x20 << 1) | 0x1A, (0x0B << 4) | 0x0C, 0x53]);
        let d = decode_object_bytes(bytes);
        assert_eq!(d.kind, ObjectEntryKind::Standard);
        assert!(d.new_screen);
        assert_eq!(d.id, 0x2B);
        assert_eq!(d.x_field, 0xC);
        assert_eq!(d.y_field, 0x1A);
        assert_eq!(d.settings, 0x53);
        // Re-encoding the decoded fields reproduces the bytes.
        assert_eq!(encode_object_bytes(d.id, d.settings, false, d.x_field, d.y_field, d.new_screen), bytes);
    }

    #[test]
    fn extended_object_round_trip() {
        let bytes = encode_object_bytes(0x7F, 0, true, 0x5, 0x1E, false);
        assert_eq!(bytes, [0x1E, 0x05, 0x7F]);
        let d = decode_object_bytes(bytes);
        assert_eq!(d.kind, ObjectEntryKind::Extended);
        assert!(!d.new_screen);
        assert_eq!(d.id, 0x7F);
        assert_eq!(d.x_field, 0x5);
        assert_eq!(d.y_field, 0x1E);
        assert_eq!(encode_object_bytes(d.id, d.settings, true, d.x_field, d.y_field, d.new_screen), bytes);
    }

    #[test]
    fn exit_and_screen_jump_classification() {
        // Exit: screen 3, midway, id 0x0123 -> bytes [0x03, 0x09, 0x00] (+ 4th byte 0x23).
        assert_eq!(classify_object_bytes([0x03, 0x09, 0x00]), ObjectEntryKind::Exit);
        // Screen jump to screen 5.
        assert_eq!(classify_object_bytes([0x05, 0x00, 0x01]), ObjectEntryKind::ScreenJump);
        // Extended id 0x02 is a real extended object, not a jump.
        assert_eq!(classify_object_bytes([0x00, 0x00, 0x02]), ObjectEntryKind::Extended);
    }

    #[test]
    fn sprite_round_trip() {
        // y=0x1B, x=0x7, screen=0x12, extra=2, id=0x35.
        let bytes = encode_sprite_bytes(0x7, 0x1B, 0x12, 2, 0x35);
        assert_eq!(bytes, [0xB0 | (1 << 1) | (2 << 2) | 1, 0x72, 0x35]);
        let d = decode_sprite_bytes(bytes);
        assert_eq!(d, DecodedSprite {
            x_tile:     0x7,
            y_tile:     0x1B,
            screen:     0x12,
            extra_bits: 2,
            sprite_id:  0x35,
        });
        assert_eq!(encode_sprite_bytes(d.x_tile, d.y_tile, d.screen, d.extra_bits, d.sprite_id), bytes);
    }

    #[test]
    fn sprite_absolute_coords_match_editor_math() {
        // Horizontal: screen 2, x 5, y 20 -> (37, 20).
        assert_eq!(sprite_absolute_coords(2, 5, 20, false), (37, 20));
        // Vertical: screen 3 -> sx=1, sy=1 -> (21, 52).
        assert_eq!(sprite_absolute_coords(3, 5, 20, true), (21, 52));
    }

    #[test]
    fn object_absolute_coords_match_editor_math() {
        // Horizontal: screen 1, x=0xC, y=0x1A -> (28, 26).
        let d = decode_object_bytes([0x9A, 0xBC, 0x53]);
        assert_eq!(object_absolute_coords(&d, 1, false), (28, 26));
        // Vertical: (y_field, screen*16 + x_field) = (26, 28).
        assert_eq!(object_absolute_coords(&d, 1, true), (26, 28));
    }

    #[test]
    fn hex_parse_accepts_lm_style_input() {
        assert_eq!(parse_hex_byte("ff"), Ok(0xFF));
        assert_eq!(parse_hex_byte("0x1a"), Ok(0x1A));
        assert_eq!(parse_hex_byte("$2B"), Ok(0x2B));
        assert_eq!(parse_hex_byte(" 7 "), Ok(0x07));
        assert!(parse_hex_byte("").is_err());
        assert!(parse_hex_byte("xyz").is_err());
        assert!(parse_hex_byte("123").is_err());
    }

    #[test]
    fn object_summary_shows_fields_and_refusals() {
        let bytes = encode_object_bytes(0x2B, 0x53, false, 0xC, 0x1A, true);
        assert_eq!(
            object_decoded_summary(bytes, false),
            Ok("Standard object 0x2B · X=0xC Y=0x1A · settings 0x53 · new-screen flag".to_string())
        );
        // Same bytes refused against an extended entry.
        assert!(object_decoded_summary(bytes, true).is_err());
        // Exit and screen-jump patterns are refused outright.
        assert!(object_decoded_summary([0x03, 0x09, 0x00], false).is_err());
        assert!(object_decoded_summary([0x05, 0x00, 0x01], false).is_err());
    }

    #[test]
    fn sprite_summary_shows_fields() {
        let bytes = encode_sprite_bytes(0x7, 0x1B, 0x12, 2, 0x35);
        assert_eq!(sprite_decoded_summary(bytes), "Sprite 0x35 · tile (7, 27) · screen 0x12 · extra bits 2");
    }

    #[test]
    fn decode_matches_smwe_rom_object_accessors() {
        // A standard object straight out of a level stream: bytes
        // [0x28, 0x35, 0x33] -> id 0x03? verify against Object methods.
        let raw = smwe_rom::objects::Object(u32::from_be_bytes([0x28, 0x35, 0x33, 0]));
        assert!(raw.is_standard());
        let d = decode_object_bytes([0x28, 0x35, 0x33]);
        assert_eq!(d.kind, ObjectEntryKind::Standard);
        assert_eq!(d.id, raw.standard_object_number());
        assert_eq!(d.x_field, raw.x());
        assert_eq!(d.y_field, raw.y());
        assert_eq!(d.settings, raw.settings());
        assert_eq!(d.new_screen, raw.is_new_screen());
    }
}
