//! Shared free-space scanning for repointing ROM data (GFX files, level
//! layer/sprite data, message boxes, overworld layer 2, ...). Previously
//! duplicated near-verbatim in `level_editor` and `world_editor`; consolidated
//! in `smwe_rom::freespace` so every write path that needs to repoint something
//! behaves the same.

pub use smwe_rom::freespace::{find_free_space, find_free_space_in};
