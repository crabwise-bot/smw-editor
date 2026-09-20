//! Lunar Magic v1.91 parity: Options > "Check Object Placement on Save".
//!
//! LM's v1.91 changelog added "an option to the Options menu called 'Check
//! Object Placement on Save'. When this option is enabled, the program will
//! check the object placement of the current level whenever you save to the
//! ROM, and warn you of any objects that were placed outside of the level
//! boundaries."
//!
//! Level bounds (tiles) match the editor's level geometry:
//! - horizontal level: width = 16 * screens, height = 27
//! - vertical level:   width = 32,            height = 16 * screens
//! where `screens` is the level-length byte + 1. The live check runs against
//! each level editor tab's current (unsaved) edit state and uses the same
//! effective screen count the save path writes (so LM 3.40 "Auto-Set Number
//! of Screens" never warns about what the save itself fixes).
//!
//! Vanilla object extents are not modeled anywhere in the editor, so — like
//! LM's documented check — this is placement-based: an item is flagged when
//! its position (or, for Direct Map16 objects whose footprint is known, any
//! part of its rectangle) lies outside the level boundaries. Screen exits
//! and screen jumps are control records, not placed objects, and are not
//! checked.

use smwe_rom::{level::Level, objects::Object};

/// Tiles per screen along the level's long axis.
pub const SCREEN_TILES: u32 = 16;
/// Horizontal levels are 27 tiles tall (the editor's render geometry).
pub const HORIZONTAL_LEVEL_HEIGHT_TILES: u32 = 27;
/// Vertical levels are 32 tiles wide.
pub const VERTICAL_LEVEL_WIDTH_TILES: u32 = 32;

/// Level bounds in tiles: `(width, height)`.
pub fn level_bounds(vertical: bool, screens: u32) -> (u32, u32) {
    if vertical {
        (VERTICAL_LEVEL_WIDTH_TILES, SCREEN_TILES * screens)
    } else {
        (SCREEN_TILES * screens, HORIZONTAL_LEVEL_HEIGHT_TILES)
    }
}

/// What kind of placed thing was flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementItemKind {
    Object,
    Sprite,
    DirectMap16,
}

/// One thing the check flagged as outside the level boundaries.
#[derive(Debug, Clone)]
pub struct PlacementIssue {
    /// Translevel (`0x000`–`0x1FF`).
    pub level:    u16,
    pub kind:     PlacementItemKind,
    /// Absolute tile position of the item's anchor.
    pub x:        u32,
    pub y:        u32,
    /// Human label, e.g. `"object $2A"`, `"sprite $7B"`, `"direct Map16 object"`.
    pub label:    String,
    /// Level orientation and screen count at check time, so the warning
    /// dialog can print the bounds without re-deriving them per level.
    pub vertical: bool,
    pub screens:  u32,
}

/// A placed item to check: absolute tile position plus extent. Vanilla
/// objects/sprites are position-only (`1×1`); Direct Map16 objects carry
/// their real `w×h` footprint.
#[derive(Debug, Clone)]
pub struct PlacedItem {
    pub x:     u32,
    pub y:     u32,
    pub w:     u32,
    pub h:     u32,
    pub kind:  PlacementItemKind,
    pub label: String,
}

impl PlacedItem {
    /// A position-only item (vanilla object or sprite).
    pub fn at(x: u32, y: u32, kind: PlacementItemKind, label: impl Into<String>) -> Self {
        PlacedItem { x, y, w: 1, h: 1, kind, label: label.into() }
    }

    /// An item with a known footprint (Direct Map16 object).
    pub fn rect(x: u32, y: u32, w: u32, h: u32, kind: PlacementItemKind, label: impl Into<String>) -> Self {
        PlacedItem { x, y, w: w.max(1), h: h.max(1), kind, label: label.into() }
    }
}

/// Flag every item whose anchor — or, for items with an extent, any part of
/// its rectangle — lies outside the level boundaries.
///
/// For sprites in vertical levels the game never reads the encoded y_tile
/// byte for Y positioning (SMWDisX `bank_02.asm` `CODE_02A93C`: the vertical
/// branch sets Y from the camera, not from byte 0), so a y derived from
/// y_tile is not a game-meaningful position. Instead the sprite's screen
/// number must exist (`screen < screens`); a sprite on a nonexistent screen
/// never loads.
pub fn check_items(level: u16, vertical: bool, screens: u32, items: &[PlacedItem]) -> Vec<PlacementIssue> {
    let (bw, bh) = level_bounds(vertical, screens);
    items
        .iter()
        .filter(|it| {
            // Vertical sprites: validate the screen number, not the
            // y_tile-derived Y (see doc comment above).
            if vertical && it.kind == PlacementItemKind::Sprite {
                let screen = (it.y / 32) * 2 + (it.x / SCREEN_TILES);
                return screen >= screens;
            }
            it.x >= bw || it.y >= bh || it.x.saturating_add(it.w) > bw || it.y.saturating_add(it.h) > bh
        })
        .map(|it| PlacementIssue { level, kind: it.kind, x: it.x, y: it.y, label: it.label.clone(), vertical, screens })
        .collect()
}

/// Heading line of the on-save warning dialog for `count` issues. Shared by
/// the UI dialog and the headless screenshot binary so the picture shows
/// exactly what the dialog says.
pub fn warning_heading(count: usize) -> String {
    format!("{count} object{} placed outside the level boundaries:", if count == 1 { " is" } else { "s are" })
}

/// Hint line shown under the issue list in the on-save warning dialog.
pub const SAVE_KEEPS_HINT: &str = "Saving keeps them where they are — they may not appear in-game.";

/// One-line dialog text for an issue. Shared by the UI dialog and the
/// headless screenshot binary so the picture shows exactly what the dialog
/// says.
pub fn format_issue(issue: &PlacementIssue) -> String {
    let kind = match issue.kind {
        PlacementItemKind::Object => "object",
        PlacementItemKind::Sprite => "sprite",
        PlacementItemKind::DirectMap16 => "direct Map16 object",
    };
    let (bw, bh) = level_bounds(issue.vertical, issue.screens);
    format!(
        "Level ${:03X}: {} {} at tile ({}, {}) is outside the level ({}×{} tiles)",
        issue.level, kind, issue.label, issue.x, issue.y, bw, bh
    )
}

/// Decode a parsed ROM level's objects and sprites into placed items, using
/// the same coordinate math as the level editor (`EditableObject::from_raw`
/// and `EditableSpriteLayer::from_rom_sprite_layer`): screen tracking via
/// new-screen bits / screen jumps, x/y swap in vertical levels.
pub fn items_of_parsed_level(level: &Level) -> Vec<PlacedItem> {
    let vertical = level.secondary_header.vertical_level();
    let mut items = Vec::new();

    if let Some(objects) = Object::parse_from_layer(level.layer1.as_bytes()) {
        let mut current_screen: u32 = 0;
        for obj in objects {
            if obj.is_exit() {
                continue;
            }
            if obj.is_screen_jump() {
                current_screen = obj.screen_number() as u32;
                continue;
            }
            if obj.is_new_screen() {
                current_screen = current_screen.saturating_add(1);
            }
            let (local_x, local_y) =
                if vertical { (obj.y() as u32, obj.x() as u32) } else { (obj.x() as u32, obj.y() as u32) };
            let abs_x = local_x + if vertical { 0 } else { current_screen * SCREEN_TILES };
            let abs_y = local_y + if vertical { current_screen * SCREEN_TILES } else { 0 };
            let label = if obj.is_extended() {
                format!("extended object ${:02X}", obj.settings())
            } else {
                format!("object ${:02X}", obj.standard_object_number())
            };
            items.push(PlacedItem::at(abs_x, abs_y, PlacementItemKind::Object, label));
        }
    }

    for spr in &level.sprite_layer.sprites {
        let (x_tile, y_tile) = spr.xy_pos();
        let screen = spr.screen_number() as u32;
        let (x, y) = if vertical {
            ((screen % 2) * SCREEN_TILES + x_tile as u32, (screen / 2) * 32 + y_tile as u32)
        } else {
            (screen * SCREEN_TILES + x_tile as u32, y_tile as u32)
        };
        items.push(PlacedItem::at(x, y, PlacementItemKind::Sprite, format!("sprite ${:02X}", spr.sprite_id())));
    }

    items
}

/// Scan one parsed ROM level for out-of-bounds placements. Used by the
/// real-ROM test; the live on-save check runs against editor tab state via
/// [`crate::ui::tool::DockableEditorTool::placement_issues`] instead, so
/// unsaved edits are checked before they are written.
pub fn check_parsed_level(level_num: u16, level: &Level) -> Vec<PlacementIssue> {
    let vertical = level.secondary_header.vertical_level();
    let screens = u32::from(level.primary_header.level_length()) + 1;
    check_items(level_num, vertical, screens, &items_of_parsed_level(level))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues_for(vertical: bool, screens: u32, items: &[PlacedItem]) -> Vec<PlacementIssue> {
        check_items(0x105, vertical, screens, items)
    }

    #[test]
    fn horizontal_bounds_edges() {
        // 2 screens = 32×27 tiles.
        let ok = [
            PlacedItem::at(0, 0, PlacementItemKind::Object, "object $2A"),
            PlacedItem::at(31, 26, PlacementItemKind::Object, "object $2A"),
            PlacedItem::at(0, 26, PlacementItemKind::Sprite, "sprite $7B"),
        ];
        assert!(issues_for(false, 2, &ok).is_empty());

        let bad = [
            PlacedItem::at(32, 0, PlacementItemKind::Object, "object $2A"), // past last screen
            PlacedItem::at(0, 27, PlacementItemKind::Object, "object $2A"), // below level (27 tall)
            PlacedItem::at(31, 27, PlacementItemKind::Sprite, "sprite $7B"),
        ];
        let issues = issues_for(false, 2, &bad);
        assert_eq!(issues.len(), 3);
        assert_eq!((issues[0].x, issues[0].y), (32, 0));
        assert_eq!((issues[1].x, issues[1].y), (0, 27));
    }

    #[test]
    fn vertical_bounds_edges() {
        // 2 screens = 32×32 tiles.
        let ok = [
            PlacedItem::at(31, 31, PlacementItemKind::Object, "object $2A"),
            PlacedItem::at(0, 0, PlacementItemKind::Sprite, "sprite $0E"),
        ];
        assert!(issues_for(true, 2, &ok).is_empty());

        let bad = [
            PlacedItem::at(32, 0, PlacementItemKind::Object, "object $2A"), // past 32-wide
            PlacedItem::at(0, 32, PlacementItemKind::Sprite, "sprite $0E"), // screen 2 of 2
        ];
        assert_eq!(issues_for(true, 2, &bad).len(), 2);
    }

    #[test]
    fn vertical_sprite_screen_is_checked_not_y_tile() {
        // The game never reads y_tile for vertical sprite Y (it uses the
        // camera), so a high y-derived Y with a valid screen must not flag.
        // This is the vanilla $0C2/$109/$12A case: e.g. (12, 91) on a
        // 5-screen level is screen 4, which exists.
        let ok = [
            PlacedItem::at(12, 91, PlacementItemKind::Sprite, "sprite $0B"), // screen 4 of 5
        ];
        assert!(issues_for(true, 5, &ok).is_empty());

        // But a sprite whose screen does not exist must flag, even if its
        // y-derived Y happens to be small. (0, 64) decodes to screen 4.
        let bad = [PlacedItem::at(0, 64, PlacementItemKind::Sprite, "sprite $0E")]; // screen 4 of 2
        assert_eq!(issues_for(true, 2, &bad).len(), 1);
    }

    #[test]
    fn direct_map16_extent_is_checked() {
        // Anchor inside, but the rectangle spills past the right edge.
        let spill = PlacedItem::rect(30, 0, 3, 2, PlacementItemKind::DirectMap16, "direct Map16 object".to_string());
        assert_eq!(issues_for(false, 2, &[spill]).len(), 1);
        // Fits exactly: x=29 w=3 → tiles 29..=31 of a 32-wide level.
        let fits = PlacedItem::rect(29, 0, 3, 2, PlacementItemKind::DirectMap16, "direct Map16 object".to_string());
        assert!(issues_for(false, 2, &[fits]).is_empty());
    }

    #[test]
    fn warning_heading_text() {
        assert_eq!(warning_heading(1), "1 object is placed outside the level boundaries:");
        assert_eq!(warning_heading(3), "3 objects are placed outside the level boundaries:");
    }

    #[test]
    fn format_issue_text() {
        let issue = PlacementIssue {
            level:    0x105,
            kind:     PlacementItemKind::Sprite,
            x:        40,
            y:        5,
            label:    "sprite $7B".to_string(),
            vertical: false,
            screens:  2,
        };
        assert_eq!(
            format_issue(&issue),
            "Level $105: sprite sprite $7B at tile (40, 5) is outside the level (32×27 tiles)"
        );
    }

    /// Real-ROM test: the vanilla game must pass the placement check, except
    /// for known quirks documented below. If a vanilla level flags beyond
    /// those quirks, the bounds or the coordinate decode is wrong.
    ///
    /// Known vanilla quirks (verified against SMWDisX):
    /// - Level $108 has three OBJLedge ($14) objects at tiles (35,0),
    ///   (38,0), (41,0) — screen 2 of a 2-screen level. The SMW object
    ///   loader (`bank_05.asm` `LoadLevelData`) increments the screen counter
    ///   on the new-screen bit unconditionally, even after a screen jump, so
    ///   these genuinely decode outside the header bounds. This is a Nintendo
    ///   quirk in the vanilla data, not a decode bug.
    #[test]
    #[ignore]
    fn real_rom_vanilla_levels_are_clean() {
        use smwe_rom::{level::LEVEL_COUNT, snes_utils::rom::Rom};

        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM");
        let rom_bytes = std::fs::read(&rom_path).expect("cannot read ROM");
        let stripped: &[u8] = if rom_bytes.len() % 0x400 == 0x200 { &rom_bytes[0x200..] } else { &rom_bytes };
        let rom = Rom::new(stripped.to_vec()).expect("parsing ROM image");
        let mut findings = Vec::new();
        for level_num in 0..LEVEL_COUNT as u16 {
            let Ok(level) = Level::parse(&rom, u32::from(level_num)) else { continue };
            for issue in check_parsed_level(level_num, &level) {
                eprintln!("finding: {}", format_issue(&issue));
                findings.push((level_num, issue.label.clone(), issue.x, issue.y));
            }
        }
        // Only the documented $108 OBJLedge quirk may appear.
        let expected = vec![
            (0x108u16, "object $14".to_string(), 35u32, 0u32),
            (0x108u16, "object $14".to_string(), 38u32, 0u32),
            (0x108u16, "object $14".to_string(), 41u32, 0u32),
        ];
        assert_eq!(findings, expected, "vanilla levels must not trip the placement check beyond known quirks");
    }
}
