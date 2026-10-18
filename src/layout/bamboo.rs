//! Note bamboo: the joint template of a compact line.
//!
//! A compact line is built from [`Joint`]s, each a bamboo joint whose stem carries
//! the repeaters and whose cap carries the note blocks. Joints stack end to end
//! into one long culm, and each is read within the row's south-bound flag, so it
//! learns the facing from its parent instead of storing it.

use super::blocks::{Facing, WireConn, chain_block, inst_block, note_block, repeater, wire_state};
use super::{Layout, air};
use crate::{BlockState, RedStoneTick};
use mcdata::util::BlockPos;
use rsnbs::note::Tone;
use std::collections::HashSet;

// Joint: the smallest unit of a row
// ++++++++++++============++++++++++++============++++++++++++============

/// One joint of a compact row: a stem carrying `stem` tick repeaters or bare
/// wire, and a cap of up to three notes.
#[derive(Clone, Copy)]
pub enum Joint {
    /// A stem topped by a repeater instead of notes, used to delay in place.
    Hold {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Ticks the cap repeater adds.
        cap: RedStoneTick,
    },
    /// Bare wire sinking three notes into this row's chain, with no delay.
    Sink {
        /// Whether the previous cap raises this wire.
        raised: bool,
        /// Notes played on the cap.
        cap: [Option<Tone>; 3],
    },
    /// Bare wire riding upward and two notes on the cap, with no delay.
    Node {
        /// Whether the previous cap raises this wire.
        raised: bool,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
    /// A delayed stem sinking three notes into this row's chain.
    DelayedSink {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 3],
    },
    /// A delayed stem riding upward and two notes on the cap.
    DelayedNode {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
}

impl Joint {
    // Constructors...

    /// A joint holding its cap repeater for `cap` extra ticks.
    pub fn hold(stem: RedStoneTick, cap: RedStoneTick) -> Self {
        Self::Hold { stem, cap }
    }

    /// A joint sinking three notes into the row's chain.
    ///
    /// A delayed stem is all repeaters, so `raised` only bears on a bare wire.
    pub fn sink(stem: RedStoneTick, cap: [Option<Tone>; 3], raised: bool) -> Self {
        match stem {
            0 => Self::Sink { raised, cap },
            stem => Self::DelayedSink { stem, cap },
        }
    }

    /// A joint riding the wire into the next row.
    ///
    /// A delayed stem is all repeaters, so `raised` only bears on a bare wire.
    pub fn node(stem: RedStoneTick, cap: [Option<Tone>; 2], raised: bool) -> Self {
        match stem {
            0 => Self::Node { raised, cap },
            stem => Self::DelayedNode { stem, cap },
        }
    }

    // Query methods...

    /// Ticks the stem delays.
    pub fn stem(&self) -> RedStoneTick {
        match *self {
            Joint::Sink { .. } | Joint::Node { .. } => 0,
            Joint::Hold { stem, .. }
            | Joint::DelayedSink { stem, .. }
            | Joint::DelayedNode { stem, .. } => stem,
        }
    }

    /// Whether the previous cap raises the bare wire.
    pub fn raised(&self) -> bool {
        match *self {
            Joint::Sink { raised, .. } | Joint::Node { raised, .. } => raised,
            _ => unreachable!(),
        }
    }

    /// The bare wire a joint rides: a node's cap climbs onto its own wire, while
    /// a sink's cap meets it flat.
    fn wire(&self, south_bound: bool) -> BlockState {
        use WireConn::*;
        let forward = match *self {
            Self::Node { .. } => Up,
            Self::Sink { .. } => Side,
            _ => unreachable!(),
        };
        let backward = if self.raised() { Up } else { Side };
        let (north, south) = match south_bound {
            true => (backward, forward),
            false => (forward, backward),
        };
        wire_state(Idle, Idle, north, south, "0")
    }
}

impl Layout<bool> for Joint {
    fn block_at_with(&self, pos: BlockPos, &south_bound: &bool) -> Option<BlockState> {
        use {Facing::*, Joint::*, WireConn::*};
        let local_z = if south_bound { pos.z } else { 1 - pos.z };
        let facing = || if south_bound { South } else { North };
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), facing(), false, false);
        debug_assert!(!matches!(self, Hold { stem: 0, .. } | Hold { cap: 0, .. }));

        let stem = || match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) if self.stem() > 0 => Some(repeater(self.stem())),
            (1, 1) => Some(self.wire(south_bound)),
            _ => None,
        };
        let hold = |cap: RedStoneTick| match (pos.x, pos.y) {
            (1, 0) => Some(chain_block()),
            (1, 1) => Some(repeater(cap)),
            _ => None,
        };
        let node = |cap: [Option<Tone>; 2]| match (pos.x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(wire_state(Idle, Idle, Side, Side, "0")),
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
            (1, Node { cap, .. } | DelayedNode { cap, .. }) => node(cap),
            (1, Sink { cap, .. } | DelayedSink { cap, .. }) => sink(cap),
            _ => unreachable!(),
        }
    }

    fn size_with(&self, _ctx: &bool) -> BlockPos {
        BlockPos::new(3, 3, 2)
    }

    /// The blocks a joint can draw: its stem and cap repeaters, the wire it rides
    /// on, and one note and instrument block per stored tone.
    fn block_palette_with(&self, _ctx: &bool) -> HashSet<BlockState> {
        todo!()
    }
}
