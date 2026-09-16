//! ROM-wide cross-reference index ("find all references" for level data).
//!
//! Answers questions like "which levels use sprite 0x3F?" or "which levels
//! play music track 0x05?" by summarizing each parsed [`Level`] once and then
//! serving lookups from the summary. Strictly read-only: it never writes to
//! the ROM, and it only uses data the existing parsers already expose.

use std::collections::BTreeSet;

use crate::level::{
    object_layer::{ExtendedInstance, ObjectInstance},
    Layer2Data,
    Level,
    ObjectLayer,
};

// -------------------------------------------------------------------------------------------------

/// Per-level usage summary. All ID sets are sorted (BTreeSet) so results are
/// deterministic.
#[derive(Debug, Clone, Default)]
pub struct LevelRefs {
    /// Sprite IDs placed in this level ([`SpriteLayer`](crate::level::SpriteLayer)).
    pub sprites:          BTreeSet<u8>,
    /// Standard object IDs placed in this level.
    pub standard_objects: BTreeSet<u8>,
    /// Extended object IDs placed in this level (exit/screen-jump commands
    /// excluded — they carry destinations, not object numbers).
    pub extended_objects: BTreeSet<u8>,
    /// Layer-2 background tile IDs (only for levels whose Layer 2 is a
    /// background, not objects).
    pub background_tiles: BTreeSet<u8>,
    /// Music track from the primary header (0-7).
    pub music:            u8,
    /// Level numbers reachable through this level's normal exit objects.
    /// Secondary exits are intentionally excluded: they point at secondary
    /// entrances, not levels.
    pub exits_to:         BTreeSet<u16>,
}

// -------------------------------------------------------------------------------------------------

/// What a cross-reference query looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrefQuery {
    Sprite(u8),
    StandardObject(u8),
    ExtendedObject(u8),
    BackgroundTile(u8),
    Music(u8),
    ExitDestination(u16),
}

// -------------------------------------------------------------------------------------------------

/// In-memory cross-reference index over a slice of parsed levels, addressed
/// by level number (0..`levels.len()`).
#[derive(Debug, Clone, Default)]
pub struct XrefIndex {
    per_level: Vec<LevelRefs>,
}

impl XrefIndex {
    /// Summarize every level in `levels`. The index is positional: entry `i`
    /// describes `levels[i]`.
    pub fn build(levels: &[Level]) -> Self {
        Self { per_level: levels.iter().map(LevelRefs::from_level).collect() }
    }

    /// Number of levels in the index.
    pub fn level_count(&self) -> usize {
        self.per_level.len()
    }

    /// Usage summary for one level, or `None` if the level number is out of range.
    pub fn refs_for(&self, level_num: u16) -> Option<&LevelRefs> {
        self.per_level.get(level_num as usize)
    }

    /// Levels using the given sprite ID.
    pub fn levels_using_sprite(&self, sprite_id: u8) -> Vec<u16> {
        self.matching(|refs| refs.sprites.contains(&sprite_id))
    }

    /// Levels using the given standard object ID.
    pub fn levels_using_standard_object(&self, object_id: u8) -> Vec<u16> {
        self.matching(|refs| refs.standard_objects.contains(&object_id))
    }

    /// Levels using the given extended object ID.
    pub fn levels_using_extended_object(&self, object_id: u8) -> Vec<u16> {
        self.matching(|refs| refs.extended_objects.contains(&object_id))
    }

    /// Levels whose Layer-2 background references the given tile ID.
    pub fn levels_using_background_tile(&self, tile_id: u8) -> Vec<u16> {
        self.matching(|refs| refs.background_tiles.contains(&tile_id))
    }

    /// Levels playing the given music track.
    pub fn levels_using_music(&self, track: u8) -> Vec<u16> {
        self.matching(|refs| refs.music == track)
    }

    /// Levels with a normal exit leading to `level_num`.
    pub fn levels_exiting_to(&self, level_num: u16) -> Vec<u16> {
        self.matching(|refs| refs.exits_to.contains(&level_num))
    }

    /// Run a query and return the matching level numbers, ascending.
    pub fn search(&self, query: XrefQuery) -> Vec<u16> {
        match query {
            XrefQuery::Sprite(id) => self.levels_using_sprite(id),
            XrefQuery::StandardObject(id) => self.levels_using_standard_object(id),
            XrefQuery::ExtendedObject(id) => self.levels_using_extended_object(id),
            XrefQuery::BackgroundTile(id) => self.levels_using_background_tile(id),
            XrefQuery::Music(track) => self.levels_using_music(track),
            XrefQuery::ExitDestination(level) => self.levels_exiting_to(level),
        }
    }

    fn matching(&self, pred: impl Fn(&LevelRefs) -> bool) -> Vec<u16> {
        self.per_level.iter().enumerate().filter(|(_, refs)| pred(refs)).map(|(i, _)| i as u16).collect()
    }
}

// -------------------------------------------------------------------------------------------------

impl LevelRefs {
    fn from_level(level: &Level) -> Self {
        let mut refs = LevelRefs { music: level.primary_header.music(), ..Default::default() };

        for sprite in &level.sprite_layer.sprites {
            refs.sprites.insert(sprite.sprite_id());
        }

        let mut index_objects = |layer: &ObjectLayer| {
            for object in layer.objects() {
                match object {
                    ObjectInstance::Standard(std) => {
                        refs.standard_objects.insert(std.std_obj_num());
                        if let Some(ext) = std.ext_obj_num() {
                            refs.extended_objects.insert(ext);
                        }
                    }
                    ObjectInstance::Extended(ExtendedInstance::Other(other)) => {
                        refs.extended_objects.insert(other.ext_obj_num());
                    }
                    ObjectInstance::Extended(ExtendedInstance::Exit(exit)) => {
                        if !exit.secondary_exit() {
                            refs.exits_to.insert(exit.destination_level());
                        }
                    }
                    ObjectInstance::Extended(ExtendedInstance::ScreenJump(_)) => {}
                }
            }
        };
        index_objects(&level.layer1);
        if let Layer2Data::Objects { objects, .. } = &level.layer2 {
            index_objects(objects);
        }
        if let Layer2Data::Background(bg) = &level.layer2 {
            refs.background_tiles.extend(bg.tile_ids().iter().copied());
        }

        refs
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{PrimaryHeader, SecondaryHeader, SpriteHeader};

    /// Build a synthetic level: `sprite_ids` placed as sprites, `object_bytes`
    /// as the Layer-1 object stream (0xFF-terminated), `music` in the header.
    fn test_level(sprite_ids: &[u8], object_bytes: &[u8], music: u8) -> Level {
        let mut header_bytes = [0u8; 5];
        header_bytes[2] = music << 4; // music = MMM in byte 2
        let sprite_stream: Vec<u8> = sprite_ids
            .iter()
            .flat_map(|&id| [0x10u8, 0x20, id]) // x/y position bytes, then sprite ID
            .chain(std::iter::once(0xFF))
            .collect();

        Level {
            primary_header:   PrimaryHeader::new(&header_bytes),
            secondary_header: SecondaryHeader([0, 0, 0, 0]),
            sprite_header:    SpriteHeader(0),
            layer1:           crate::level::ObjectLayer::parse(object_bytes).unwrap().1 .0,
            layer2:           Layer2Data::Objects {
                header:  [0u8; crate::level::LAYER2_HEADER_SIZE],
                objects: crate::level::ObjectLayer::parse(&[0xFF]).unwrap().1 .0,
            },
            sprite_layer:     crate::level::SpriteLayer::parse(&sprite_stream).unwrap().1 .0,
        }
    }

    /// Standard object instance bytes: std_obj_num() = ((byte0 >> 1) & 0b110000)
    /// | ((byte1 >> 4) & 0b1111), so BB comes from byte0 bits 6-5.
    fn std_obj(num_hi_bits: u8, num_lo_bits: u8, settings: u8) -> [u8; 3] {
        [0x80 | ((num_hi_bits & 0b11) << 5), (num_lo_bits & 0b1111) << 4, settings]
    }

    #[test]
    fn sprite_lookup_finds_levels_using_it() {
        // Level 0: sprites 0x3F and 0x01. Level 1: sprite 0x01 only. Level 2: none.
        let levels =
            vec![test_level(&[0x3F, 0x01], &[0xFF], 0), test_level(&[0x01], &[0xFF], 0), test_level(&[], &[0xFF], 0)];
        let index = XrefIndex::build(&levels);

        assert_eq!(index.levels_using_sprite(0x3F), vec![0]);
        assert_eq!(index.levels_using_sprite(0x01), vec![0, 1]);
        assert!(index.levels_using_sprite(0x7F).is_empty());
    }

    #[test]
    fn query_dispatch_matches_individual_lookups() {
        let levels = vec![test_level(&[0x3F], &[0xFF], 5)];
        let index = XrefIndex::build(&levels);

        assert_eq!(index.search(XrefQuery::Sprite(0x3F)), vec![0]);
        assert_eq!(index.search(XrefQuery::Music(5)), vec![0]);
        assert!(index.search(XrefQuery::Music(1)).is_empty());
    }

    #[test]
    fn standard_and_extended_objects_are_indexed() {
        // 0x14 standard object + an extended object (first two bytes zero,
        // third byte 0x42) on layer 1.
        let mut bytes = std_obj(0b01, 0b0100, 0x00).to_vec();
        bytes.extend_from_slice(&[0x00, 0x00, 0x42, 0xFF]);
        let levels = vec![test_level(&[], &bytes, 0)];
        let index = XrefIndex::build(&levels);

        assert_eq!(index.levels_using_standard_object(0x14), vec![0]);
        assert_eq!(index.levels_using_extended_object(0x42), vec![0]);
        assert!(index.levels_using_standard_object(0x15).is_empty());
    }

    #[test]
    fn layer2_objects_are_indexed_too() {
        let mut level = test_level(&[], &[0xFF], 0);
        // Extended object 0x77 placed on layer 2 (objects variant).
        let l2_bytes = [0x00u8, 0x00, 0x77, 0xFF];
        level.layer2 = Layer2Data::Objects {
            header:  [0u8; crate::level::LAYER2_HEADER_SIZE],
            objects: crate::level::ObjectLayer::parse(&l2_bytes).unwrap().1 .0,
        };
        let index = XrefIndex::build(std::slice::from_ref(&level));

        assert_eq!(index.levels_using_extended_object(0x77), vec![0]);
        // Layer-2 backgrounds contribute no object IDs.
        assert!(index.levels_using_extended_object(0x42).is_empty());
    }

    #[test]
    fn background_tiles_are_indexed_for_bg_layer2() {
        let mut level = test_level(&[], &[0xFF], 0);
        // Minimal RLE1 stream: direct-copy 3 bytes [0xA1, 0xB2, 0xC3], then terminator.
        let compressed = [0x02u8, 0xA1, 0xB2, 0xC3, 0xFF];
        let (bg, _) = crate::level::BackgroundData::read_from(&compressed).unwrap();
        level.layer2 = Layer2Data::Background(bg);
        let index = XrefIndex::build(std::slice::from_ref(&level));

        assert_eq!(index.levels_using_background_tile(0xB2), vec![0]);
        assert!(index.levels_using_background_tile(0x00).is_empty());
    }

    #[test]
    fn music_tracks_are_indexed_per_level() {
        let levels = vec![test_level(&[], &[0xFF], 5), test_level(&[], &[0xFF], 5), test_level(&[], &[0xFF], 2)];
        let index = XrefIndex::build(&levels);

        assert_eq!(index.levels_using_music(5), vec![0, 1]);
        assert_eq!(index.levels_using_music(2), vec![2]);
        assert!(index.levels_using_music(0).is_empty());
    }

    #[test]
    fn exit_destinations_are_indexed() {
        // Exit object: [screen=0x00, flags=0x00 (normal exit), ext_num=0x00, dest_lo=0x05]
        // -> destination_level() = 0x005.
        let bytes = [0x00u8, 0x00, 0x00, 0x05, 0xFF];
        let levels = vec![test_level(&[], &bytes, 0), test_level(&[], &[0xFF], 0)];
        let index = XrefIndex::build(&levels);

        assert_eq!(index.levels_exiting_to(5), vec![0]);
        assert!(index.levels_exiting_to(6).is_empty());
        // An exit is not an extended object: it must not show up there.
        assert!(index.levels_using_extended_object(0x00).is_empty());
    }

    #[test]
    fn refs_for_returns_none_out_of_range() {
        let levels = vec![test_level(&[0x3F], &[0xFF], 3)];
        let index = XrefIndex::build(&levels);

        let refs = index.refs_for(0).unwrap();
        assert!(refs.sprites.contains(&0x3F));
        assert_eq!(refs.music, 3);
        assert!(index.refs_for(1).is_none());
        assert_eq!(index.level_count(), 1);
    }
}
