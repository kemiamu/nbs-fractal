//! Note bamboo: the joint template of a compact line.
//!
//! A compact line is built from [`Joint`]s, each a bamboo joint whose stem carries
//! the repeaters and whose cap carries the note blocks. Joints stack end to end
//! into one long culm, and each is read within the [`Frame`] of the row that owns
//! it, so it learns the row width and facing from its parent instead of storing
//! them.

use super::blocks::{Facing, chain_block, inst_block, note_block, redstone_wire, repeater};
use super::{Layout, air};
use crate::{BlockState, RedStoneTick};
use mcdata::util::BlockPos;
use rsnbs::note::Tone;
use std::collections::HashSet;

/// The row-level context every joint in a row is read within: the width it spans
/// and which way it faces.
pub struct Frame {
    /// Width of the row in blocks.
    pub width: i32,
    /// Whether the row runs south-bound rather than north-bound.
    pub south_bound: bool,
}

/// One joint of a compact row: a stem of `stem` tick repeaters and a cap of up
/// to three notes.
#[derive(Clone, Copy)]
pub enum Joint {
    /// A stem topped by a repeater instead of notes, used to delay in place.
    Hold {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Ticks the cap repeater adds.
        cap: RedStoneTick,
    },
    /// A stem feeding this row's chain and three notes on the cap.
    Sink {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 3],
    },
    /// A stem riding the wire upward and two notes on the cap.
    Node {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
}

impl Joint {
    /// Ticks the stem delays.
    pub fn stem(&self) -> RedStoneTick {
        match *self {
            Joint::Hold { stem, .. } | Joint::Sink { stem, .. } | Joint::Node { stem, .. } => stem,
        }
    }

    /// A joint holding its cap repeater for `cap` extra ticks.
    pub fn hold(stem: RedStoneTick, cap: RedStoneTick) -> Self {
        Self::Hold { stem, cap }
    }

    /// A joint sinking three notes into the row's chain.
    pub fn sink(stem: RedStoneTick, cap: [Option<Tone>; 3]) -> Self {
        Self::Sink { stem, cap }
    }

    /// A joint riding the wire into the next row.
    pub fn node(stem: RedStoneTick, cap: [Option<Tone>; 2]) -> Self {
        Self::Node { stem, cap }
    }
}

impl Layout<Frame> for Joint {
    fn block_at_with(&self, pos: BlockPos, frame: &Frame) -> Option<BlockState> {
        use self::{Facing::*, Joint::*};
        let south_bound = frame.south_bound;
        let local_z = if south_bound { pos.z } else { 1 - pos.z };
        let facing = if south_bound { South } else { North };
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), facing, false, false);
        debug_assert!(!matches!(self, Hold { stem: 0, .. } | Hold { cap: 0, .. }));

        let stem = || match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) if self.stem() == 0 => Some(redstone_wire()),
            (1, 1) => Some(repeater(self.stem())),
            _ => None,
        };
        let hold = |cap: RedStoneTick| match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) => Some(repeater(cap)),
            _ => None,
        };
        let node = |cap: [Option<Tone>; 2]| match (pos.x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(redstone_wire()),
            (0, 0) => Some(inst_block(cap[0], air)),
            (0, 1) => Some(note_block(cap[0], air)),
            (2, 0) => Some(inst_block(cap[1], air)),
            (2, 1) => Some(note_block(cap[1], air)),
            _ => None,
        };
        let sink = |cap: [Option<Tone>; 3]| match (pos.x, pos.y) {
            (1, 0) => Some(inst_block(cap[0], chain_block)),
            (1, 1) => Some(note_block(cap[0], chain_block)),
            (0, 0) => Some(inst_block(cap[1], air)),
            (0, 1) => Some(note_block(cap[1], air)),
            (2, 0) => Some(inst_block(cap[2], air)),
            (2, 1) => Some(note_block(cap[2], air)),
            _ => None,
        };
        match (local_z, *self) {
            (0, _) => stem(),
            (1, Hold { cap, .. }) => hold(cap),
            (1, Node { cap, .. }) => node(cap),
            (1, Sink { cap, .. }) => sink(cap),
            _ => unreachable!(),
        }
    }

    fn size_with(&self, _frame: &Frame) -> BlockPos {
        BlockPos::new(3, 3, 2)
    }

    /// The blocks a joint can draw: its stem and cap repeaters, the wire it rides
    /// on, and one note and instrument block per stored tone.
    fn block_palette_with(&self, _frame: &Frame) -> HashSet<BlockState> {
        todo!()
    }
}
