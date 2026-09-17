#![allow(clippy::identity_op)]
#![allow(dead_code)]

use anyhow::{anyhow, bail, Result};
use smwe_rom::{direct_map16, level::Level, objects::Object};

use crate::undo::Undo;

const SCREEN_WIDTH: u32 = 16;

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::ui::editor_prototypes) struct EditableObject {
    pub x:           u32,
    pub y:           u32,
    pub id:          u8,
    pub settings:    u8,
    pub is_extended: bool,
    pub extended_id: u8,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::ui::editor_prototypes) struct EditableExit {
    pub screen:    u8,
    pub midway:    bool,
    pub secondary: bool,
    pub id:        u16,
}

#[derive(Clone, Debug, Default)]
pub(in crate::ui::editor_prototypes) struct EditableObjectLayer {
    pub objects: Vec<EditableObject>,
    pub exits:   Vec<EditableExit>,
}

impl EditableObject {
    pub fn from_raw(object: Object, current_screen: u32, vertical_level: bool) -> Option<Self> {
        if object.is_exit() || object.is_screen_jump() {
            return None;
        }

        let (local_x, local_y) = if vertical_level {
            (object.y() as u32, object.x() as u32)
        } else {
            (object.x() as u32, object.y() as u32)
        };

        let abs_x = local_x + if vertical_level { 0 } else { current_screen * SCREEN_WIDTH };
        let abs_y = local_y + if vertical_level { current_screen * SCREEN_WIDTH } else { 0 };

        Some(EditableObject {
            x:           abs_x,
            y:           abs_y,
            id:          object.standard_object_number(),
            settings:    object.settings(),
            is_extended: object.is_extended(),
            extended_id: object.settings(),
        })
    }

    pub fn to_raw(self, new_screen: bool) -> Object {
        if self.is_extended {
            Object(
                ((new_screen as u32) << 31)
                    | ((self.y & 0x1F) << 24)
                    | ((self.x & 0x0F) << 16)
                    | (self.extended_id as u32),
            )
        } else {
            Object(
                ((new_screen as u32) << 31)
                    | ((self.id as u32 & 0x30) << 30)
                    | ((self.id as u32 & 0x0F) << 20)
                    | ((self.y & 0x1F) << 24)
                    | ((self.x & 0x0F) << 16)
                    | (self.settings as u32),
            )
        }
    }
}

impl EditableExit {
    pub fn from_raw(object: Object) -> Option<Self> {
        object.is_exit().then(|| EditableExit {
            screen:    object.screen_number(),
            midway:    object.is_midway(),
            secondary: object.is_secondary_exit(),
            id:        object.exit_id(),
        })
    }

    pub fn to_raw(self) -> Object {
        Object(
            ((self.screen as u32 & 0x1F) << 24)
                | ((self.midway as u32) << 19)
                | ((self.secondary as u32) << 17)
                | (self.id as u32 & 0x3FFF),
        )
    }
}

impl EditableObjectLayer {
    pub fn from_level(level: &Level) -> Self {
        Self::from_object_layer(&level.layer1, level.secondary_header.vertical_level())
    }

    pub fn from_object_layer(layer_src: &smwe_rom::level::ObjectLayer, is_vertical: bool) -> Self {
        let raw_bytes = layer_src.as_bytes();
        let raw_objects = match Object::parse_from_layer(raw_bytes) {
            Some(objs) => objs,
            None => return Self::default(),
        };
        let mut layer = Self::default();
        let mut current_screen: u8 = 0;
        for raw_object in raw_objects {
            if raw_object.is_exit() {
                if let Some(exit) = EditableExit::from_raw(raw_object) {
                    layer.exits.push(exit);
                }
            } else if raw_object.is_screen_jump() {
                current_screen = raw_object.screen_number();
            } else {
                if raw_object.is_new_screen() {
                    current_screen = current_screen.saturating_add(1);
                }
                if let Some(obj) = EditableObject::from_raw(raw_object, current_screen as u32, is_vertical) {
                    layer.objects.push(obj);
                }
            }
        }
        layer
    }

    pub fn serialize_layer1_bytes(&self, vertical_level: bool) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.objects.len() * 3 + self.exits.len() * 4 + 8);
        let mut current_screen = 0u8;

        for obj in &self.objects {
            let (target_screen, raw_x, raw_y) = obj.screen_and_local_coords(vertical_level)?;
            if target_screen != current_screen {
                if target_screen == current_screen.saturating_add(1) {
                    current_screen = target_screen;
                    bytes.extend_from_slice(&obj.raw_bytes(true, raw_x, raw_y));
                } else {
                    bytes.extend_from_slice(&screen_jump_bytes(target_screen));
                    current_screen = target_screen;
                    bytes.extend_from_slice(&obj.raw_bytes(false, raw_x, raw_y));
                }
            } else {
                bytes.extend_from_slice(&obj.raw_bytes(false, raw_x, raw_y));
            }
        }

        for exit in &self.exits {
            bytes.extend_from_slice(&exit.to_raw_bytes());
        }

        bytes.push(0xFF);
        Ok(bytes)
    }

    /// The 3 raw stream bytes for the object at `index`, computed exactly the
    /// way `serialize_layer1_bytes` lays them out (screen tracking + the
    /// new-screen flag), so the Edit Manual dialog shows the bytes as they
    /// exist in the level data.
    pub fn stream_bytes_for_index(&self, index: usize, vertical_level: bool) -> Option<[u8; 3]> {
        self.objects.get(index)?;
        let mut current_screen = 0u8;
        for (i, o) in self.objects.iter().enumerate() {
            let (target_screen, raw_x, raw_y) = o.screen_and_local_coords(vertical_level).ok()?;
            if i == index {
                let new_screen = target_screen == current_screen.saturating_add(1);
                return Some(o.raw_bytes(new_screen, raw_x, raw_y));
            }
            if target_screen != current_screen {
                current_screen = target_screen;
            }
        }
        None
    }
}

impl EditableObject {
    /// (screen, raw local X, raw local Y) of this object for stream encoding.
    /// Public so the Edit Manual dialog can recover the object's screen when
    /// applying edited bytes.
    pub fn screen_and_local_coords(&self, vertical_level: bool) -> Result<(u8, u8, u8)> {
        if vertical_level {
            let screen = u8::try_from(self.y / SCREEN_WIDTH)
                .map_err(|_| anyhow!("vertical object screen out of range: {}", self.y / SCREEN_WIDTH))?;
            let raw_x = u8::try_from(self.y % SCREEN_WIDTH).unwrap();
            let raw_y = u8::try_from(self.x).map_err(|_| anyhow!("vertical object x out of range: {}", self.x))?;
            if raw_y > 0x1F {
                bail!("vertical object x out of range for raw format: {}", self.x);
            }
            Ok((screen, raw_x, raw_y))
        } else {
            let screen = u8::try_from(self.x / SCREEN_WIDTH)
                .map_err(|_| anyhow!("horizontal object screen out of range: {}", self.x / SCREEN_WIDTH))?;
            let raw_x = u8::try_from(self.x % SCREEN_WIDTH).unwrap();
            let raw_y = u8::try_from(self.y).map_err(|_| anyhow!("horizontal object y out of range: {}", self.y))?;
            if raw_y > 0x1F {
                bail!("horizontal object y out of range for raw format: {}", self.y);
            }
            Ok((screen, raw_x, raw_y))
        }
    }

    fn raw_bytes(&self, new_screen: bool, raw_x: u8, raw_y: u8) -> [u8; 3] {
        if self.is_extended {
            [(u8::from(new_screen) << 7) | (raw_y & 0x1F), raw_x & 0x0F, self.extended_id]
        } else {
            [
                (u8::from(new_screen) << 7) | ((self.id & 0x30) << 1) | (raw_y & 0x1F),
                ((self.id & 0x0F) << 4) | (raw_x & 0x0F),
                self.settings,
            ]
        }
    }
}

impl EditableExit {
    fn to_raw_bytes(self) -> [u8; 4] {
        [
            self.screen & 0x1F,
            (u8::from(self.midway) << 3) | (u8::from(self.secondary) << 1) | ((self.id >> 8) as u8 & 0x3F),
            0,
            self.id as u8,
        ]
    }
}

fn screen_jump_bytes(screen: u8) -> [u8; 3] {
    [screen & 0x1F, 0, 1]
}

impl Undo for EditableObjectLayer {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        if bytes.len() < 4 {
            return Self::default();
        }
        let num_objects = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        let num_exits = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;

        let mut objects = Vec::with_capacity(num_objects);
        let mut offset = 4;
        for _ in 0..num_objects {
            if offset + 12 > bytes.len() {
                break;
            }
            let x = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let y = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            let id = bytes[offset + 8];
            let settings = bytes[offset + 9];
            let is_extended = bytes[offset + 10] != 0;
            let extended_id = bytes[offset + 11];
            objects.push(EditableObject { x, y, id, settings, is_extended, extended_id });
            offset += 12;
        }

        let mut exits = Vec::with_capacity(num_exits);
        for _ in 0..num_exits {
            if offset + 5 > bytes.len() {
                break;
            }
            let screen = bytes[offset];
            let midway = bytes[offset + 1] != 0;
            let secondary = bytes[offset + 2] != 0;
            let id = u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]);
            exits.push(EditableExit { screen, midway, secondary, id });
            offset += 5;
        }

        Self { objects, exits }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + self.objects.len() * 12 + self.exits.len() * 5);
        bytes.extend_from_slice(&(self.objects.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(self.exits.len() as u16).to_le_bytes());
        for obj in &self.objects {
            bytes.extend_from_slice(&obj.x.to_le_bytes());
            bytes.extend_from_slice(&obj.y.to_le_bytes());
            bytes.push(obj.id);
            bytes.push(obj.settings);
            bytes.push(obj.is_extended as u8);
            bytes.push(obj.extended_id);
        }
        for exit in &self.exits {
            bytes.push(exit.screen);
            bytes.push(exit.midway as u8);
            bytes.push(exit.secondary as u8);
            bytes.extend_from_slice(&exit.id.to_le_bytes());
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        4 + self.objects.len() * 12 + self.exits.len() * 5
    }
}

#[cfg(test)]
mod tests {
    use super::{EditableExit, EditableObject, EditableObjectLayer};

    #[test]
    fn stream_bytes_for_index_matches_serialized_stream() {
        let layer = EditableObjectLayer {
            objects: vec![
                EditableObject {
                    x:           0,
                    y:           0,
                    id:          0x12,
                    settings:    0x34,
                    is_extended: false,
                    extended_id: 0,
                },
                EditableObject {
                    x:           17,
                    y:           5,
                    id:          0x2F,
                    settings:    0x56,
                    is_extended: false,
                    extended_id: 0,
                },
                // Extended entry back on screen 0: the serialized stream
                // inserts a screen jump before it.
                EditableObject {
                    x:           3,
                    y:           2,
                    id:          0,
                    settings:    0,
                    is_extended: true,
                    extended_id: 0x7F,
                },
            ],
            exits:   vec![],
        };
        // Per-entry stream bytes include the new-screen flag exactly as the
        // serializer lays them out.
        assert_eq!(layer.stream_bytes_for_index(0, false), Some([0x20, 0x20, 0x34]));
        assert_eq!(layer.stream_bytes_for_index(1, false), Some([0xC5, 0xF1, 0x56]));
        assert_eq!(layer.stream_bytes_for_index(2, false), Some([0x02, 0x03, 0x7F]));
        assert_eq!(layer.stream_bytes_for_index(3, false), None);
        // And they sit at the same positions in the full serialized stream
        // (modulo the screen-jump entry before the extended object).
        assert_eq!(layer.serialize_layer1_bytes(false).unwrap(), vec![
            0x20, 0x20, 0x34, 0xC5, 0xF1, 0x56, 0x00, 0x00, 0x01, 0x02, 0x03, 0x7F, 0xFF
        ]);
    }

    #[test]
    fn serializes_horizontal_objects_and_exits() {
        let layer = EditableObjectLayer {
            objects: vec![
                EditableObject {
                    x:           0,
                    y:           0,
                    id:          0x12,
                    settings:    0x34,
                    is_extended: false,
                    extended_id: 0,
                },
                EditableObject {
                    x:           17,
                    y:           5,
                    id:          0x2F,
                    settings:    0x56,
                    is_extended: false,
                    extended_id: 0,
                },
            ],
            exits:   vec![EditableExit { screen: 3, midway: false, secondary: false, id: 0x0123 }],
        };

        let bytes = layer.serialize_layer1_bytes(false).unwrap();

        assert_eq!(bytes, vec![0x20, 0x20, 0x34, 0xC5, 0xF1, 0x56, 0x03, 0x01, 0x00, 0x23, 0xFF]);
    }

    #[test]
    fn serializes_vertical_objects_with_screen_jump() {
        let layer = EditableObjectLayer {
            objects: vec![
                EditableObject {
                    x:           7,
                    y:           18,
                    id:          0x11,
                    settings:    0xAA,
                    is_extended: false,
                    extended_id: 0,
                },
                EditableObject {
                    x:           9,
                    y:           64,
                    id:          0x22,
                    settings:    0xBB,
                    is_extended: false,
                    extended_id: 0,
                },
            ],
            exits:   vec![],
        };

        let bytes = layer.serialize_layer1_bytes(true).unwrap();

        assert_eq!(bytes, vec![0xA7, 0x12, 0xAA, 0x04, 0x00, 0x01, 0x49, 0x20, 0xBB, 0xFF]);
    }
}

// -------------------------------------------------------------------------------------------------
// Direct Map16 objects (LM v1.70-v1.90 "Add Objects / Direct Map16" parity)
// -------------------------------------------------------------------------------------------------

/// One editable Direct Map16 object: a rectangular Map16 pattern placed as a
/// single resizable level object. Distinct from vanilla objects — stored in
/// this editor's native RATS block (see `smwe_rom::direct_map16`), never in
/// the vanilla object stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::ui::editor_prototypes) struct EditableDm16Object {
    /// Absolute level tile coords of the object's top-left corner.
    pub x:         u32,
    pub y:         u32,
    /// Current object size in tiles; the pattern repeats to fill it.
    pub w:         u32,
    pub h:         u32,
    /// Stored pattern dimensions in tiles.
    pub pw:        u32,
    pub ph:        u32,
    /// Row-major Map16 block IDs (`pw * ph` entries).
    pub tiles:     Vec<u16>,
    /// Optional conditional flag: `(ram_addr, bit)` — bit 0..=7, 8 = nonzero.
    /// Inert metadata on a stock ROM (see `direct_map16` docs).
    pub condition: Option<(u16, u8)>,
}

impl EditableDm16Object {
    pub fn from_rom_obj(o: &direct_map16::DirectMap16Object) -> Self {
        EditableDm16Object {
            x:         o.x,
            y:         o.y,
            w:         o.w,
            h:         o.h,
            pw:        o.pw,
            ph:        o.ph,
            tiles:     o.tiles.clone(),
            condition: o.condition.map(|c| (c.ram_addr, c.bit)),
        }
    }

    pub fn to_rom_obj(&self) -> direct_map16::DirectMap16Object {
        direct_map16::DirectMap16Object {
            x:         self.x,
            y:         self.y,
            w:         self.w,
            h:         self.h,
            pw:        self.pw,
            ph:        self.ph,
            tiles:     self.tiles.clone(),
            condition: self.condition.map(|(ram_addr, bit)| direct_map16::DirectMap16Condition { ram_addr, bit }),
        }
    }

    /// The Map16 tile rendered at local coords (lx, ly): the pattern repeats
    /// when the object is larger than the pattern.
    pub fn tile_at(&self, lx: u32, ly: u32) -> u16 {
        if self.tiles.is_empty() || self.pw == 0 || self.ph == 0 {
            return 0x25;
        }
        self.tiles[((ly % self.ph) * self.pw + (lx % self.pw)) as usize]
    }

    /// Whether absolute tile (tx, ty) is covered by this object.
    pub fn covers(&self, tx: u32, ty: u32) -> bool {
        tx >= self.x && ty >= self.y && tx < self.x + self.w && ty < self.y + self.h
    }

    /// Clamp dimensions into 1..=64 and fix up the tile vector after a
    /// resize (pattern keeps its tiles; the object size repeats them).
    pub fn sanitize(&mut self) {
        self.w = self.w.clamp(1, direct_map16::DM16_MAX_DIM);
        self.h = self.h.clamp(1, direct_map16::DM16_MAX_DIM);
        self.pw = self.pw.clamp(1, direct_map16::DM16_MAX_DIM);
        self.ph = self.ph.clamp(1, direct_map16::DM16_MAX_DIM);
        self.tiles.resize((self.pw * self.ph) as usize, 0x25);
        if let Some((_, bit)) = &mut self.condition {
            *bit = (*bit).min(8);
        }
    }
}

/// The level's Direct Map16 objects, kept separate from the vanilla object
/// layer (they serialize to the editor-native RATS block, not the object
/// stream).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::ui::editor_prototypes) struct EditableDirectMap16 {
    pub objects: Vec<EditableDm16Object>,
}

impl EditableDirectMap16 {
    pub fn from_rom_data(data: &direct_map16::DirectMap16Data, level: u16) -> Self {
        EditableDirectMap16 { objects: data.objects_for(level).iter().map(EditableDm16Object::from_rom_obj).collect() }
    }
}

impl Undo for EditableDirectMap16 {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        if bytes.len() < 2 {
            return Self::default();
        }
        let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        let mut objects = Vec::with_capacity(count.min(1024));
        let mut off = 2usize;
        for _ in 0..count {
            if off + 13 > bytes.len() {
                break;
            }
            let x = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
            let y = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let w = bytes[off + 8] as u32;
            let h = bytes[off + 9] as u32;
            let pw = bytes[off + 10] as u32;
            let ph = bytes[off + 11] as u32;
            let cond_present = bytes[off + 12] != 0;
            off += 13;
            let condition = if cond_present {
                if off + 3 > bytes.len() {
                    break;
                }
                let ram_addr = u16::from_le_bytes([bytes[off], bytes[off + 1]]);
                let bit = bytes[off + 2].min(8);
                off += 3;
                Some((ram_addr, bit))
            } else {
                None
            };
            let tile_count = (pw.max(1).min(64) as usize) * (ph.max(1).min(64) as usize);
            if off + tile_count * 2 > bytes.len() {
                break;
            }
            let mut tiles = Vec::with_capacity(tile_count);
            for i in 0..tile_count {
                tiles.push(u16::from_le_bytes([bytes[off + 2 * i], bytes[off + 2 * i + 1]]));
            }
            off += tile_count * 2;
            let mut obj = EditableDm16Object { x, y, w, h, pw, ph, tiles, condition };
            obj.sanitize();
            objects.push(obj);
        }
        Self { objects }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.objects.len().min(1024) as u16).to_le_bytes());
        for obj in self.objects.iter().take(1024) {
            bytes.extend_from_slice(&obj.x.to_le_bytes());
            bytes.extend_from_slice(&obj.y.to_le_bytes());
            bytes.push(obj.w.clamp(1, 64) as u8);
            bytes.push(obj.h.clamp(1, 64) as u8);
            bytes.push(obj.pw.clamp(1, 64) as u8);
            bytes.push(obj.ph.clamp(1, 64) as u8);
            match obj.condition {
                None => bytes.push(0),
                Some((ram_addr, bit)) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&ram_addr.to_le_bytes());
                    bytes.push(bit.min(8));
                }
            }
            // Keep the stream self-consistent: exactly the declared pattern
            // size is written (truncate oversized vectors, pad undersized).
            let want = (obj.pw.clamp(1, 64) as usize) * (obj.ph.clamp(1, 64) as usize);
            for t in obj.tiles.iter().take(want) {
                bytes.extend_from_slice(&t.to_le_bytes());
            }
            for _ in obj.tiles.len().min(want)..want {
                bytes.extend_from_slice(&0x25u16.to_le_bytes());
            }
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        2 + self
            .objects
            .iter()
            .map(|o| {
                let want = (o.pw.clamp(1, 64) as usize) * (o.ph.clamp(1, 64) as usize);
                13 + if o.condition.is_some() { 3 } else { 0 } + want * 2
            })
            .sum::<usize>()
    }
}

#[cfg(test)]
mod dm16_tests {
    use super::EditableDirectMap16;
    use crate::undo::Undo;

    #[test]
    fn dm16_undo_round_trip() {
        let layer = EditableDirectMap16 {
            objects: vec![
                super::EditableDm16Object {
                    x:         10,
                    y:         20,
                    w:         4,
                    h:         3,
                    pw:        2,
                    ph:        2,
                    tiles:     vec![0x25, 0x26, 0x2B, 0x2C],
                    condition: Some((0x14AF, 0)),
                },
                super::EditableDm16Object {
                    x:         0,
                    y:         0,
                    w:         1,
                    h:         1,
                    pw:        1,
                    ph:        1,
                    tiles:     vec![0x130],
                    condition: None,
                },
            ],
        };
        let bytes = layer.to_bytes();
        assert_eq!(bytes.len(), layer.size_bytes());
        let back = EditableDirectMap16::from_bytes(bytes);
        assert_eq!(back, layer);
        // Pattern repetition on the decoded object.
        assert_eq!(back.objects[0].tile_at(2, 0), 0x25);
        assert_eq!(back.objects[0].tile_at(3, 2), 0x26);
    }

    #[test]
    fn dm16_from_bytes_tolerates_truncation() {
        let back = EditableDirectMap16::from_bytes(vec![0x02, 0x00, 0xFF]);
        assert!(back.objects.is_empty());
    }
}
