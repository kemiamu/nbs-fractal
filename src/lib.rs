//! Fractal structures of NBS songs.
//!
//! This crate builds on [`rsnbs`]: [`analysis`] decomposes the note plane
//! into translation equivalence classes, [`layout`] arranges the notes into
//! projection layouts, and [`schematic`] writes them out as litematic files.

pub mod analysis;
pub mod layout;
pub mod schematic;

#[cfg(test)]
mod tests;

use rsnbs::note::{Instrument, Tone};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// Redstone tick (10 t/s).
pub type RedStoneTick = rsnbs::types::Tick;
/// Game tick (20 t/s).
pub type GameTick = rsnbs::types::Tick;

// Block state
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A block state as a name and a sorted map of properties.
///
/// Properties stay sorted so equal states hash and compare alike, letting a
/// state key a palette without a hand-written hash.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockState {
    /// The id of this block, e.g. `minecraft:air`.
    pub name: Cow<'static, str>,

    /// The properties of this block state, as a map from names to values.
    pub properties: BTreeMap<Cow<'static, str>, Cow<'static, str>>,
}

impl BlockState {
    /// `minecraft:air`, the empty block state.
    pub fn air() -> Self {
        Self {
            name: "minecraft:air".into(),
            properties: BTreeMap::new(),
        }
    }
}

impl From<mcdata::GenericBlockState> for BlockState {
    /// Rebuilds a `mcdata` state into this crate's own sorted form.
    fn from(state: mcdata::GenericBlockState) -> Self {
        Self {
            name: state.name,
            properties: BTreeMap::from_iter(state.properties),
        }
    }
}

impl From<BlockState> for mcdata::GenericBlockState {
    /// Rebuilds this crate's own state into a `mcdata` state.
    fn from(state: BlockState) -> Self {
        Self {
            name: state.name,
            properties: HashMap::from_iter(state.properties),
        }
    }
}

// util
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Keeps only the notes that render into a note block, dropping the ones that
/// cannot. A note renders when it carries a vanilla instrument whose key lies
/// within the note block's f#3-f#5 range; mob head and custom instruments, and
/// keys outside that range, do not.
pub fn playable<I, T>(notes: I) -> impl Iterator<Item = T>
where
    I: IntoIterator<Item = T>,
    T: AsRef<Tone>,
{
    use Instrument::*;
    notes.into_iter().filter(|note| {
        let tone = note.as_ref();
        !matches!(tone.instrument, Imitate(_) | Custom(_)) && tone.key.minecraft_note().is_some()
    })
}
