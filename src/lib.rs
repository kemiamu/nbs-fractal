//! Fractal structures of NBS songs.
//!
//! This crate builds on [`rsnbs`]: [`analysis`] decomposes the note plane
//! into translation equivalence classes, and [`schematic`] reconstructs
//! Minecraft litematic projections from them.

use rsnbs::note::Note;
use rsnbs::song::Layer;
use rsnbs::types::{LayerAnchor, Panning, Volume};
use std::iter;

pub mod analysis;
pub mod schematic;

#[cfg(test)]
mod tests;

/// Redstone tick (10 t/s).
pub type RedStoneTick = rsnbs::types::Tick;
/// Game tick (20 t/s).
pub type GameTick = rsnbs::types::Tick;

/// Blends each note with its layer's volume and stereo, yielding the notes as
/// they are actually heard.
pub fn heard<A, I>(notes: I, layers: &[Layer]) -> impl Iterator<Item = (A, Note)>
where
    A: LayerAnchor,
    I: IntoIterator<Item = (A, Note)>,
{
    let mut notes = notes.into_iter();
    iter::from_fn(move || {
        let (anchor, mut note) = notes.next()?;
        let layer = &layers[usize::try_from(anchor.into_layer()).unwrap()];
        let v = i32::from(note.velocity.get());
        let n = i32::from(note.panning.get());
        let p = i32::from(layer.panning.get());
        let panning = if p == 0 { n } else { (p + n) / 2 };
        let volume = i32::from(layer.volume.get()) * v / 100;
        note.velocity = Volume::new(u8::try_from(volume).unwrap()).unwrap();
        note.panning = Panning::new(i8::try_from(panning).unwrap()).unwrap();
        Some((anchor, note))
    })
}
