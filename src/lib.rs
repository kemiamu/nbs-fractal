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

/// Redstone tick (10 t/s).
pub type RedStoneTick = rsnbs::types::Tick;
/// Game tick (20 t/s).
pub type GameTick = rsnbs::types::Tick;

// util
//
// ++++++++++++============++++++++++++============++++++++++++============

use rsnbs::note::{Instrument, Tone};

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
