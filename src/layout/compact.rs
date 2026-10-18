//! Compact note block layouts for NBS song projection.

use super::bamboo::{Events, Joint, State};
use super::blocks::{Facing, WireConn, chain_block, inst_block, note_block, repeater, wire_state};
use super::{Clipped, EvenlyArranged, Layout, Overlaid, Provided, WithFloor, air};
use crate::{BlockState, RedStoneTick, forward_layout};
use mcdata::util::BlockPos;
use rsnbs::note::Tone;
use std::collections::HashSet;
use std::iter;
use std::num::NonZero;

// Shell: MultiCompactLayout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Compact variable-length packed structure: one packed line per track, stacked
/// vertically.
pub struct MultiCompactLayout(EvenlyArranged<WithFloor<CompactLayout>>);

impl MultiCompactLayout {
    /// Create a multi-track compact layout from multiple redstone note groups,
    /// one [`CompactLayout`] line per group.
    pub fn new<Trks, Trk, T>(
        tracks: Trks,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
        full: bool,
    ) -> Self
    where
        Trks: IntoIterator<Item = (Trk, Option<NonZero<RedStoneTick>>)>,
        Trk: IntoIterator<Item = (RedStoneTick, T)>,
        T: Into<Tone>,
    {
        let layers = tracks.into_iter().map(|(notes, coarse)| {
            let line = CompactLayout::new(notes, coarse, wrap_length, gap);
            WithFloor::new(line, full)
        });
        Self(EvenlyArranged::new(layers, BlockPos::new(0, 4, 0), &()))
    }
}

forward_layout!(MultiCompactLayout, ());

// Layout: CompactLayout & Row
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One track's compact line: a variable-length packed structure with no wasted
/// gaps.
pub struct CompactLayout(Clipped<EvenlyArranged<Row>>);

impl CompactLayout {
    pub fn new<I: IntoIterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        notes: I,
        repeater_coarse: Option<NonZero<RedStoneTick>>,
        wrap_length: Option<NonZero<usize>>,
        gap: u32,
    ) -> Self {
        let width = 4 + gap as i32;
        let coarse = repeater_coarse.map_or(RedStoneTick::MAX, NonZero::get);
        let mut events = Events::new(notes.into_iter().peekable(), RedStoneTick::MAX);
        let mut south_bound = false;
        let rows = iter::from_fn(move || {
            south_bound = !south_bound;
            Row::new(&mut events, width, coarse, wrap_length, south_bound)
        });
        let rows = EvenlyArranged::new(rows, BlockPos::new(width - 2, 0, 0), &());
        Self(Clipped::new(rows, BlockPos::new(width - 3, 0, 0), &()))
    }
}

forward_layout!(CompactLayout, ());

/// The row-level context every joint and turn in a row is read within: the width
/// it spans and which way it faces.
struct Frame {
    width: i32,
    south_bound: bool,
}

/// One directional row of joints, read within the [`Frame`] it holds.
struct Row(Provided<Frame, Overlaid<Turn, EvenlyArranged<Joint>>>);

impl Row {
    fn new<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        events: &mut Events<I>,
        width: i32,
        coarse: RedStoneTick,
        wrap_length: Option<NonZero<usize>>,
        south_bound: bool,
    ) -> Option<Self> {
        let len = wrap_length.map_or(usize::MAX, NonZero::get);
        let (turn, mut state) = Turn::next(events, coarse, len == 1)?;
        let joints: Vec<Joint> = (1..len)
            .map_while(|column| Joint::next(events, &mut state, coarse, column + 1 == len))
            .collect();

        let depth = wrap_length.map_or(joints.len() + 1, NonZero::get);
        let size = BlockPos::new(width, 3, 2 * depth as i32);
        let frame = Frame { width, south_bound };
        let pitch = if south_bound { 2 } else { -2 };
        let rest = EvenlyArranged::new(joints, BlockPos::new(0, 0, pitch), &frame);
        let turn_at = BlockPos::new(0, 0, if south_bound { 0 } else { -1 });
        let joints_at = BlockPos::new(-1, 0, if south_bound { 1 } else { -2 });
        let row = Overlaid::new(turn_at, turn, joints_at, rest, size, &frame);
        Some(Self(Provided::new(frame, row)))
    }
}

forward_layout!(Row, ());

impl Layout<Frame> for Joint {
    fn block_at_with(&self, pos: BlockPos, frame: &Frame) -> Option<BlockState> {
        Layout::block_at_with(self, pos, &frame.south_bound)
    }

    fn size_with(&self, frame: &Frame) -> BlockPos {
        Layout::size_with(self, &frame.south_bound)
    }

    fn block_palette_with(&self, frame: &Frame) -> HashSet<BlockState> {
        Layout::block_palette_with(self, &frame.south_bound)
    }
}

// Template: Turn
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The turn that leads a row on from the previous one: a stem of bare wire or a
/// repeater, and a cap of up to two notes.
#[derive(Clone, Copy)]
enum Turn {
    /// Bare wire sinking two notes into this row's chain.
    Sink {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Notes played on the cap.
        cap: [Option<Tone>; 2],
    },
    /// Bare wire riding upward and one note on the cap.
    Node {
        /// Ticks in the stem.
        stem: RedStoneTick,
        /// Note played on the cap.
        cap: Option<Tone>,
    },
}

impl Turn {
    /// Draws the next turn from `events` and seeds the input state of the joints
    /// that follow, or returns `None` when the stream has no notes left.
    fn next<I: Iterator<Item = (RedStoneTick, T)>, T: Into<Tone>>(
        events: &mut Events<I>,
        coarse: RedStoneTick,
        closing: bool,
    ) -> Option<(Self, State)> {
        let (wait, count) = events.pending()?;
        let stem = wait.min(coarse.min(4));
        let feeds = closing && (wait > stem || count > 2 || events.has_next_group());
        let terminal = !feeds && (wait > 0 || count <= 2);
        events.take_waits(stem);
        let turn = match (terminal, wait > stem) {
            (true, true) => Self::sink(stem, [None; 2]),
            (true, false) => Self::sink(stem, events.take_notes(count)),
            (false, true) => Self::node(stem, None),
            (false, false) => Self::node(stem, Some(events.take_note())),
        };
        let state = State {
            terminal: matches!(turn, Self::Sink { .. }),
            raised: matches!(turn, Self::Node { .. }),
            chained: false,
        };
        Some((turn, state))
    }

    /// A turn sinking two notes into the row's chain.
    fn sink(stem: RedStoneTick, cap: [Option<Tone>; 2]) -> Self {
        Self::Sink { stem, cap }
    }

    /// A turn riding the wire into the next row.
    fn node(stem: RedStoneTick, cap: Option<Tone>) -> Self {
        Self::Node { stem, cap }
    }

    /// Ticks the stem delays.
    fn stem(&self) -> RedStoneTick {
        match *self {
            Self::Sink { stem, .. } | Self::Node { stem, .. } => stem,
        }
    }

    /// The wire a turn rides at `local_x`: its cap climbs onto the wire at the
    /// inside end, while the outside end climbs onto the previous row.
    fn wire(&self, local_x: i32, frame: &Frame) -> BlockState {
        use WireConn::*;
        let is_node = matches!(self, Self::Node { .. });
        let inside = if is_node { Up } else { Side };
        let west_end = frame.width - 1;
        let (west, east, forward) = match local_x {
            2 => (Side, inside, Idle),
            x if x == west_end => (Idle, Side, Up),
            _ => (Side, Side, Idle),
        };
        let (north, south) = match frame.south_bound {
            true => (Idle, forward),
            false => (forward, Idle),
        };
        wire_state(west, east, north, south, "0")
    }

    /// The raised wire of a node cap: it meets the turn's own wire on the west
    /// and the row's first joint on the inside.
    fn node_wire(frame: &Frame) -> BlockState {
        use WireConn::*;
        let (north, south) = match frame.south_bound {
            true => (Idle, Side),
            false => (Side, Idle),
        };
        wire_state(Side, Idle, north, south, "0")
    }
}

impl Layout<Frame> for Turn {
    fn block_at_with(&self, pos: BlockPos, frame: &Frame) -> Option<BlockState> {
        use self::{Facing::*, Turn::*};
        let local_x = frame.width - 1 - pos.x;
        let repeater = |delay: RedStoneTick| repeater(delay.to_string(), East, false, false);

        let stem = || match (local_x, pos.y) {
            (2.., 0) => Some(chain_block()),
            (2, 1) if self.stem() > 0 => Some(repeater(self.stem())),
            (2.., 1) => Some(self.wire(local_x, frame)),
            _ => None,
        };
        let sink = |cap: [Option<Tone>; 2]| match (local_x, pos.y) {
            (1, 0) => Some(inst_block(cap[0], chain_block)),
            (1, 1) => Some(note_block(cap[0], chain_block)),
            (0, 0) => Some(inst_block(cap[1], air)),
            (0, 1) => Some(note_block(cap[1], air)),
            _ => None,
        };
        let node = |cap: Option<Tone>| match (local_x, pos.y) {
            (1, 0 | 1) => Some(chain_block()),
            (1, 2) => Some(Self::node_wire(frame)),
            (0, 0) => Some(inst_block(cap, air)),
            (0, 1) => Some(note_block(cap, air)),
            _ => None,
        };
        match (local_x, *self) {
            (2.., _) => stem(),
            (0 | 1, Sink { cap, .. }) => sink(cap),
            (0 | 1, Node { cap, .. }) => node(cap),
            _ => unreachable!(),
        }
    }

    fn size_with(&self, frame: &Frame) -> BlockPos {
        BlockPos::new(frame.width, 3, 1)
    }

    /// The blocks a turn can draw: its stem repeater, the wire it rides on, and
    /// one note and instrument block per stored tone.
    fn block_palette_with(&self, frame: &Frame) -> HashSet<BlockState> {
        use Facing::East;
        let mut palette = HashSet::from([chain_block()]);
        for local_x in 2..=frame.width - 1 {
            palette.insert(self.wire(local_x, frame));
        }
        if matches!(self, Self::Node { .. }) {
            palette.insert(Self::node_wire(frame));
        }
        if self.stem() > 0 {
            palette.insert(repeater(self.stem().to_string(), East, false, false));
        }
        let caps: &[Option<Tone>] = match self {
            Self::Sink { cap, .. } => cap,
            Self::Node { cap, .. } => std::slice::from_ref(cap),
        };
        for tone in caps.iter().flatten() {
            palette.insert(note_block(Some(tone), air));
            palette.insert(inst_block(Some(tone), air));
        }
        palette
    }
}
