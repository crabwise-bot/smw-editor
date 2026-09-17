//! Cross-editor clipboard: Lunar Magic-style cut/copy/paste through the
//! system clipboard.
//!
//! Lunar Magic v2.30 copies tiles/objects through the Windows clipboard as
//! text ("made it possible to copy FG tiles from the Map16 editor into the
//! level editor through the Windows clipboard"); v1.63 added clipboard
//! copy/paste in the 16x16 and 8x8 editors, and v3.30 copies tile hex values
//! as text for pasting into ExAnimated Frames dialogs. This module is the
//! shared piece every editor uses: a versioned, human-readable text format
//! (`smwclip:1:...`) plus encode/decode and the thin egui system-clipboard
//! glue (`copy_text` / `requested_paste` / `Event::Paste`).
//!
//! The format is deliberately plain text, like LM's: a copied payload reads
//! as hex values, so it can be inspected — or a tile number lifted out of it
//! — in any text field. Payload kinds:
//!
//! - `objects` — level objects + sprites, positions relative to the
//!   selection's top-left, each object carrying its rendered footprint
//!   blocks so a paste stamps identical tiles.
//! - `map16` — rectangular region of Map16 block IDs (row-major hex).
//! - `map16words` — one Map16 block's four tile words (Block Editor).
//! - `tile8x8` — one 8x8 tile's 64 color indices + source GFX file.
//! - `owtiles` — overworld layer-1 tile IDs, rectangular region.

/// Magic prefix + format version. Bump the version if the grammar changes.
const MAGIC: &str = "smwclip";
const VERSION: &str = "1";

/// One level object on the clipboard, positioned relative to the
/// selection's top-left (`dx`, `dy` in tiles).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipObject {
    pub dx:          i32,
    pub dy:          i32,
    pub id:          u8,
    pub settings:    u8,
    pub is_extended: bool,
    pub extended_id: u8,
    /// Rendered footprint, row-major (`w * h` block IDs), so a paste can
    /// stamp the same tiles the source showed.
    pub w:           u32,
    pub h:           u32,
    pub blocks:      Vec<u16>,
}

/// One sprite on the clipboard, positioned relative to the selection's
/// top-left (`dx`, `dy` in tiles).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSprite {
    pub dx:         i32,
    pub dy:         i32,
    pub sprite_id:  u8,
    pub extra_bits: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardPayload {
    /// Level editor: objects + sprites.
    LevelObjects { objects: Vec<ClipObject>, sprites: Vec<ClipSprite> },
    /// Map16 grid: rectangular region of block IDs, row-major.
    Map16Blocks { cols: u32, rows: u32, ids: Vec<u16> },
    /// Map16 Block Editor: one block's four tile words.
    Map16BlockWords { words: [u16; 4] },
    /// 8x8 tile editor: one tile's color indices + source GFX file number.
    Tile8x8 { file: u8, pixels: [u8; 64] },
    /// Overworld editor: layer-1 tile IDs, rectangular region, row-major.
    OverworldTiles { cols: u32, rows: u32, ids: Vec<u8> },
}

fn hex_u16(v: u16) -> String {
    format!("{v:04X}")
}

fn parse_hex_u16(s: &str) -> Option<u16> {
    u16::from_str_radix(s.trim(), 16).ok()
}

fn parse_dec<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
}

impl ClipboardPayload {
    /// Encode the payload as `smwclip:1:<kind>:...` text.
    pub fn encode(&self) -> String {
        match self {
            ClipboardPayload::LevelObjects { objects, sprites } => {
                let objs = objects
                    .iter()
                    .map(|o| {
                        let blocks: String = o.blocks.iter().map(|b| hex_u16(*b)).collect::<Vec<_>>().join("");
                        format!(
                            "o,{},{},{},{},{},{},{},{},{blocks}",
                            o.dx,
                            o.dy,
                            o.id,
                            o.settings,
                            u8::from(o.is_extended),
                            o.extended_id,
                            o.w,
                            o.h,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(";");
                let sprs = sprites
                    .iter()
                    .map(|s| format!("s,{},{},{},{}", s.dx, s.dy, s.sprite_id, s.extra_bits))
                    .collect::<Vec<_>>()
                    .join(";");
                format!("{MAGIC}:{VERSION}:objects:{objs}|{sprs}")
            }
            ClipboardPayload::Map16Blocks { cols, rows, ids } => {
                let list = ids.iter().map(|id| hex_u16(*id)).collect::<Vec<_>>().join(",");
                format!("{MAGIC}:{VERSION}:map16:{cols},{rows}:{list}")
            }
            ClipboardPayload::Map16BlockWords { words } => {
                let list = words.iter().map(|w| hex_u16(*w)).collect::<Vec<_>>().join(",");
                format!("{MAGIC}:{VERSION}:map16words:{list}")
            }
            ClipboardPayload::Tile8x8 { file, pixels } => {
                let hex: String = pixels.iter().map(|p| format!("{p:02X}")).collect();
                format!("{MAGIC}:{VERSION}:tile8x8:{file}:{hex}")
            }
            ClipboardPayload::OverworldTiles { cols, rows, ids } => {
                let list = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
                format!("{MAGIC}:{VERSION}:owtiles:{cols},{rows}:{list}")
            }
        }
    }

    /// Decode `smwclip:1:...` text back into a payload. Returns `None` for
    /// anything that isn't our format (including other apps' text).
    pub fn decode(text: &str) -> Option<ClipboardPayload> {
        let text = text.trim();
        let mut parts = text.splitn(4, ':');
        if parts.next()? != MAGIC || parts.next()? != VERSION {
            return None;
        }
        let kind = parts.next()?;
        let rest = parts.next().unwrap_or("");
        match kind {
            "objects" => decode_objects(rest),
            "map16" => decode_map16(rest),
            "map16words" => decode_map16words(rest),
            "tile8x8" => decode_tile8x8(rest),
            "owtiles" => decode_owtiles(rest),
            _ => None,
        }
    }
}

fn decode_objects(rest: &str) -> Option<ClipboardPayload> {
    let (obj_part, spr_part) = rest.split_once('|').unwrap_or((rest, ""));
    let mut objects = Vec::new();
    if !obj_part.is_empty() {
        for item in obj_part.split(';') {
            // o,dx,dy,id,settings,ext,eid,w,h,blocks
            let f: Vec<&str> = item.split(',').collect();
            if f.len() != 10 || f[0] != "o" {
                return None;
            }
            let (w, h) = (parse_dec::<u32>(f[7])?, parse_dec::<u32>(f[8])?);
            let blocks_hex = f[9];
            if blocks_hex.len() != (w as usize) * (h as usize) * 4 {
                return None;
            }
            let mut blocks = Vec::with_capacity((w * h) as usize);
            for chunk in blocks_hex.as_bytes().chunks(4) {
                let s = std::str::from_utf8(chunk).ok()?;
                blocks.push(parse_hex_u16(s)?);
            }
            objects.push(ClipObject {
                dx: parse_dec(f[1])?,
                dy: parse_dec(f[2])?,
                id: parse_dec(f[3])?,
                settings: parse_dec(f[4])?,
                is_extended: parse_dec::<u8>(f[5])? != 0,
                extended_id: parse_dec(f[6])?,
                w,
                h,
                blocks,
            });
        }
    }
    let mut sprites = Vec::new();
    if !spr_part.is_empty() {
        for item in spr_part.split(';') {
            let f: Vec<&str> = item.split(',').collect();
            if f.len() != 5 || f[0] != "s" {
                return None;
            }
            sprites.push(ClipSprite {
                dx:         parse_dec(f[1])?,
                dy:         parse_dec(f[2])?,
                sprite_id:  parse_dec(f[3])?,
                extra_bits: parse_dec(f[4])?,
            });
        }
    }
    if objects.is_empty() && sprites.is_empty() {
        return None;
    }
    Some(ClipboardPayload::LevelObjects { objects, sprites })
}

fn decode_map16(rest: &str) -> Option<ClipboardPayload> {
    let (dims, list) = rest.split_once(':')?;
    let (cols, rows) = dims.split_once(',')?;
    let (cols, rows) = (parse_dec::<u32>(cols)?, parse_dec::<u32>(rows)?);
    let ids: Option<Vec<u16>> = list.split(',').map(parse_hex_u16).collect();
    let ids = ids?;
    if ids.len() != (cols as usize) * (rows as usize) || ids.is_empty() {
        return None;
    }
    Some(ClipboardPayload::Map16Blocks { cols, rows, ids })
}

fn decode_map16words(rest: &str) -> Option<ClipboardPayload> {
    let words: Option<Vec<u16>> = rest.split(',').map(parse_hex_u16).collect();
    let words = words?;
    if words.len() != 4 {
        return None;
    }
    Some(ClipboardPayload::Map16BlockWords { words: [words[0], words[1], words[2], words[3]] })
}

fn decode_tile8x8(rest: &str) -> Option<ClipboardPayload> {
    let (file, hex) = rest.split_once(':')?;
    let file: u8 = parse_dec(file)?;
    if hex.len() != 128 {
        return None;
    }
    let mut pixels = [0u8; 64];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        pixels[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(ClipboardPayload::Tile8x8 { file, pixels })
}

fn decode_owtiles(rest: &str) -> Option<ClipboardPayload> {
    let (dims, list) = rest.split_once(':')?;
    let (cols, rows) = dims.split_once(',')?;
    let (cols, rows) = (parse_dec::<u32>(cols)?, parse_dec::<u32>(rows)?);
    let ids: Option<Vec<u8>> = list.split(',').map(parse_dec).collect();
    let ids = ids?;
    if ids.len() != (cols as usize) * (rows as usize) || ids.is_empty() {
        return None;
    }
    Some(ClipboardPayload::OverworldTiles { cols, rows, ids })
}

// ── egui system-clipboard glue ──────────────────────────────────────────

/// Paste-anchor math shared by the UI and headless tools: a
/// selection-relative `(dx, dy)` offset becomes absolute tile coords, clamped
/// so a `w×h` footprint stays inside a `(level_w, level_h)` tile area.
pub fn place_footprint(anchor: (u32, u32), dx: i32, dy: i32, w: u32, h: u32, level_w: u32, level_h: u32) -> (u32, u32) {
    let nx = (anchor.0 as i32 + dx).clamp(0, level_w.saturating_sub(w) as i32).max(0) as u32;
    let ny = (anchor.1 as i32 + dy).clamp(0, level_h.saturating_sub(h) as i32).max(0) as u32;
    (nx, ny)
}

/// Copy a payload to the system clipboard as text.
pub fn copy_payload(ctx: &egui::Context, payload: &ClipboardPayload) {
    ctx.copy_text(payload.encode());
}

/// Ask the integration to fetch the system clipboard. The text arrives as an
/// `Event::Paste` on the next frame — drain it with `take_paste_text`.
/// (Ctrl+V needs no request: the integration pushes `Event::Paste` directly.)
pub fn request_paste(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

/// Drain a pending `Event::Paste` (the answer to an earlier
/// `request_paste`). Returns the pasted text, if the system has answered
/// yet. Callers gate this on their own pending flag so a text widget's
/// paste is never stolen.
pub fn take_paste_text(ctx: &egui::Context) -> Option<String> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(t) => Some(t.clone()),
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_objects() -> ClipboardPayload {
        ClipboardPayload::LevelObjects {
            objects: vec![
                ClipObject {
                    dx:          0,
                    dy:          0,
                    id:          0x12,
                    settings:    0x34,
                    is_extended: false,
                    extended_id: 0,
                    w:           2,
                    h:           1,
                    blocks:      vec![0x0100, 0x0101],
                },
                ClipObject {
                    dx:          3,
                    dy:          -1,
                    id:          0x2F,
                    settings:    0x00,
                    is_extended: true,
                    extended_id: 0x2A,
                    w:           1,
                    h:           1,
                    blocks:      vec![0x25],
                },
            ],
            sprites: vec![ClipSprite { dx: 1, dy: 2, sprite_id: 0x0F, extra_bits: 2 }],
        }
    }

    #[test]
    fn objects_roundtrip() {
        let p = sample_objects();
        let text = p.encode();
        assert!(text.starts_with("smwclip:1:objects:"));
        assert_eq!(ClipboardPayload::decode(&text), Some(p));
    }

    #[test]
    fn objects_only_sprites_roundtrip() {
        let p = ClipboardPayload::LevelObjects {
            objects: vec![],
            sprites: vec![ClipSprite { dx: 0, dy: 0, sprite_id: 1, extra_bits: 0 }],
        };
        assert_eq!(ClipboardPayload::decode(&p.encode()), Some(p));
    }

    #[test]
    fn map16_roundtrip() {
        let p = ClipboardPayload::Map16Blocks { cols: 2, rows: 2, ids: vec![0x0102, 0x00FF, 0x1234, 0x0000] };
        let text = p.encode();
        assert_eq!(text, "smwclip:1:map16:2,2:0102,00FF,1234,0000");
        assert_eq!(ClipboardPayload::decode(&text), Some(p));
    }

    #[test]
    fn map16words_roundtrip() {
        let p = ClipboardPayload::Map16BlockWords { words: [0x0027, 0x4027, 0x8000, 0xC123] };
        assert_eq!(ClipboardPayload::decode(&p.encode()), Some(p));
    }

    #[test]
    fn tile8x8_roundtrip() {
        let mut pixels = [0u8; 64];
        for (i, p) in pixels.iter_mut().enumerate() {
            *p = (i % 16) as u8;
        }
        let p = ClipboardPayload::Tile8x8 { file: 0x14, pixels };
        let text = p.encode();
        assert!(text.starts_with("smwclip:1:tile8x8:20:"));
        assert_eq!(text.len(), "smwclip:1:tile8x8:20:".len() + 128);
        assert_eq!(ClipboardPayload::decode(&text), Some(p));
    }

    #[test]
    fn owtiles_roundtrip() {
        let p = ClipboardPayload::OverworldTiles { cols: 3, rows: 1, ids: vec![5, 0, 255] };
        assert_eq!(ClipboardPayload::decode(&p.encode()), Some(p));
    }

    #[test]
    fn decode_rejects_non_payloads() {
        for bad in [
            "",
            "hello world",
            "smwclip:2:objects:o,0,0,1,2,0,0,1,1,0025|",
            "smwclip:1:nope:x",
            "smwclip:1:map16:2,2:0102",
            "smwclip:1:map16words:0102,00FF",
            "smwclip:1:tile8x8:20:ZZ",
            "smwclip:1:owtiles:2,2:1,2,3",
            // Empty selections carry nothing.
            "smwclip:1:objects:|",
        ] {
            assert_eq!(ClipboardPayload::decode(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn decode_tolerates_surrounding_whitespace() {
        let p = ClipboardPayload::Map16Blocks { cols: 1, rows: 1, ids: vec![0x25] };
        assert_eq!(ClipboardPayload::decode(&format!("  {}\n", p.encode())), Some(p));
    }

    #[test]
    fn objects_text_is_human_readable_hex() {
        // LM copies "tile hex values as text": the payload must read as hex.
        let p = ClipboardPayload::Map16Blocks { cols: 1, rows: 1, ids: vec![0x0102] };
        assert!(p.encode().contains("0102"));
    }

    #[test]
    fn place_footprint_clamps_into_level() {
        assert_eq!(place_footprint((10, 10), 0, 0, 2, 2, 32, 32), (10, 10));
        assert_eq!(place_footprint((10, 10), 3, -4, 2, 2, 32, 32), (13, 6));
        // Footprint stays fully inside: clamped to level - size.
        assert_eq!(place_footprint((30, 30), 0, 0, 4, 4, 32, 32), (28, 28));
        // Negative offsets clamp to 0.
        assert_eq!(place_footprint((0, 0), -5, -5, 1, 1, 32, 32), (0, 0));
        // Degenerate level dims don't underflow.
        assert_eq!(place_footprint((5, 5), 0, 0, 1, 1, 0, 0), (0, 0));
    }
}
