//! Cascade layout for NBS song projection.

use super::{AsLayout, EvenlyArranged, Facing, Layout, Overlaid, ToneBlocks, WireConn};
use super::{air, chain_block, inst_block, note_block, repeater, wire_state};
use mcdata::{GenericBlockState, util::BlockPos};
use rsnbs::note::{Note, Tone};
use rsnbs::types::TimeAnchor;
use std::collections::HashMap;
use std::iter;

// Layout: CascadeLayout
//
// A cascade reads as one continuous sweep rather than many independent lines:
// every tick gets its own row, and silent ticks keep their row so the wall stays
// evenly spaced. The whole sweep fires from one fixed startup, which absorbs the
// timing differences between tracks so each row can simply step forward.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A note stream projected as a cascade: one row per tick off a fixed startup.
pub struct CascadeLayout(Overlaid<Linkage, EvenlyArranged<Template>>);

impl CascadeLayout {
    /// Projects a note stream, one row per redstone tick.
    pub fn new<Trk, A, T>(notes: Trk) -> Self
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Note>,
    {
        let mut notes = notes
            .into_iter()
            .map(|(anchor, note)| (anchor.into_tick(), note.into()))
            .peekable();
        let mut rows: Vec<Template> = Vec::new();
        while let Some(&(next, _)) = notes.peek() {
            let len = (next as usize).max(rows.len());
            rows.resize_with(len, || Template::new(iter::empty()));
            let slice = iter::from_fn(|| notes.next_if(|&(t, _)| t == next).map(|(_, note)| note));
            rows.push(Template::new(slice));
        }
        if rows.is_empty() {
            rows.push(Template::new(iter::empty()));
        }

        let linkage = Linkage;
        let anchor = BlockPos::new(0, 0, linkage.size().z - 1);
        let rows = EvenlyArranged::new(rows, BlockPos::new(0, 0, 2));
        let size = BlockPos::new(rows.size().x, rows.size().y, anchor.z + rows.size().z);
        Self(Overlaid::new(BlockPos::ORIGIN, linkage, anchor, rows, size))
    }
}

impl AsLayout for CascadeLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Templates: Template & Linkage
//
// Linkage is the fixed front that sets every track running together; Template is
// one tick of the wall. Voices sit by how they are heard: loudness chooses a
// track's ring and panning divides it between the sides.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// One tick of the wall: the voices its tracks carry, three slots per track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Template {
    slots: [[Option<Tone>; 3]; 11],
}

impl Template {
    /// The three centre tracks, left inner to right inner.
    const CENTRE: [usize; 3] = [4, 5, 6];

    /// Fills a row with the voices to sound on its tick, placing each by how it is
    /// heard: loudness picks the track, panning divides the sides, and the loudest
    /// voices stack in the centre.
    fn new<I: IntoIterator<Item = Note>>(notes: I) -> Self {
        let mut template = Self {
            slots: [[None; 3]; 11],
        };
        // A voice the wall cannot sound never takes a slot; it would only push the
        // voices that do onto the sides and leave the centre bare.
        let voices = notes
            .into_iter()
            .filter(|note| note.tone.note_block_state().is_some());
        let mut centre = Vec::new();
        voices.for_each(|note| template.place(note, &mut centre));
        template.balance(&centre);
        template
    }

    /// Routes one voice: into the centre list, or onto its pair of side tracks.
    fn place(&mut self, note: Note, centre: &mut Vec<Tone>) {
        let Some(tracks) = Self::route(note.velocity.get(), note.panning.get()) else {
            centre.push(note.tone);
            return;
        };
        tracks
            .into_iter()
            .flatten()
            .for_each(|t| self.put(t, note.tone));
    }

    /// The side tracks a voice falls between, or `None` when it belongs in the centre.
    fn route(volume: u8, panning: i8) -> Option<[Option<usize>; 2]> {
        const LOUDEST: usize = 4;
        let (v, p) = (volume as i32, panning as i32);
        let ring = |l: i32| ((l + 12) / 25).clamp(0, 4) as usize;
        let (left, right) = (v * (100 - p.max(0)) / 100, v * (100 + p.min(0)) / 100);
        if ring(left) == LOUDEST && ring(right) == LOUDEST {
            return None;
        }
        let ear = |loudness: i32, west: bool| {
            let track = ring(loudness).min(LOUDEST - 1);
            (loudness > 0).then(|| if west { 10 - track } else { track })
        };
        Some([ear(left, false), ear(right, true)])
    }

    /// Routes the centre voices over the three middle tracks. The centre track
    /// takes as many as it can while staying balanced and the outer two mirror it;
    /// any overflow spills to the quieter side tracks, alternating sides.
    fn balance(&mut self, centre: &[Tone]) {
        let count = centre.len();
        let middle = match count {
            0..=3 => count,
            _ if count % 2 == 1 => 3,
            _ => 2,
        };
        let outer = ((count - middle) / 2).min(3);
        let [west, mid, east] = Self::CENTRE;
        let mut rest = centre;
        for (track, share) in [(mid, middle), (west, outer), (east, outer)] {
            let share = share.min(rest.len());
            // The centre track balances an even pair by skipping its middle; every
            // other track keeps its middle, which is what powers it.
            let order = match track == mid && share == 2 {
                true => [1, 2, 0],
                false => Self::order(track),
            };
            let (chunk, tail) = rest.split_at(share);
            Self::spread(&mut self.slots[track], chunk, order);
            rest = tail;
        }
        for (side, &tone) in rest.iter().enumerate() {
            self.put(if side % 2 == 0 { west - 1 } else { east + 1 }, tone);
        }
    }

    /// A track's slot fill order: its own middle first, then the slot toward the
    /// centre, then the one away from it, so a track reads as centred on itself
    /// while leaning in with the wall.
    fn order(track: usize) -> [usize; 3] {
        match track < Self::CENTRE[1] {
            true => [0, 2, 1],
            false => [0, 1, 2],
        }
    }

    /// Lays a track's voices onto its slots, in the given order.
    fn spread(slots: &mut [Option<Tone>; 3], tones: &[Tone], order: [usize; 3]) {
        for (&slot, &tone) in order.iter().zip(tones) {
            slots[slot] = Some(tone);
        }
    }

    /// Keeps a track's middle slot powered when it carries no voice, so the row's
    /// signal still runs through.
    fn relay(&self, y: i32, track: usize, x: i32) -> Option<GenericBlockState> {
        use WireConn::{None as Open, Side};
        const CENTRE_X: i32 = 37;
        let slots = &self.slots[track];
        let split = slots[1].is_some() || slots[2].is_some();
        let bare_heart = x == CENTRE_X && slots[0].is_none() && split;
        if y == 0 || bare_heart {
            return Some(chain_block());
        }
        Some(wire_state(Open, Open, Side, Side, "0"))
    }

    /// Records a voice, spilling outward onto the quieter tracks once its own is full.
    fn put(&mut self, track: usize, tone: Tone) {
        let step: i32 = if track <= 3 { -1 } else { 1 };
        let mut outward = iter::successors(Some(track as i32), move |&t| {
            (0..=10).contains(&(t + step)).then(|| t + step)
        });
        let _ = outward.any(|t| self.try_put(t as usize, tone));
    }

    /// Spills a voice onto a track's first free slot, in that track's fill order.
    fn try_put(&mut self, track: usize, tone: Tone) -> bool {
        let order = Self::order(track);
        let slots = &mut self.slots[track];
        let Some(&slot) = order.iter().find(|&&slot| slots[slot].is_none()) else {
            return false;
        };
        slots[slot] = Some(tone);
        true
    }
}

impl Layout for Template {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        // The tracks' centre x positions, leftmost outward track to rightmost, and
        // each track's three slots as x offsets from its centre: middle, left, right.
        const TRACK_X: [i32; 11] = [1, 5, 11, 21, 34, 37, 40, 53, 63, 69, 73];
        const SLOT_DX: [i32; 3] = [0, -1, 1];
        let cell = TRACK_X.iter().enumerate().find_map(|(track, &c)| {
            let slot = SLOT_DX.iter().position(|&dx| c + dx == pos.x)?;
            Some((track, slot))
        });
        match (pos.z, pos.y, cell) {
            // z = 0: the drive row, one repeater on each track's middle slot.
            (0, 0, Some((_, 0))) => Some(chain_block()),
            (0, 1, Some((_, 0))) => Some(repeater("1", Facing::South, false, false)),
            // z = 1: the note row. Every stored voice can sound (filtered in
            // `new`), so a bare middle slot relays rather than meeting air.
            (1, _, Some((t, 0))) if self.slots[t][0].is_none() => self.relay(pos.y, t, pos.x),
            (1, 0, Some((t, s))) => self.slots[t][s].map(|tone| inst_block(Some(tone), air)),
            (1, 1, Some((t, s))) => self.slots[t][s].map(|tone| note_block(Some(tone), air)),
            _ => None,
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(75, 2, 2)
    }
}

/// The fixed z0..=z4 startup.
struct Linkage;

impl Layout for Linkage {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        use WireConn::{None as Open, Side};
        match (pos.z, pos.y, pos.x) {
            // z0..=z2
            (0..=2, 0, 37) => Some(chain_block()),
            (0, 1, 37) => Some(button()),
            (1..=2, 1, 37) => Some(repeater("4", Facing::South, false, false)),
            // z3
            (3, 1, 6 | 22) => Some(repeater("1", Facing::West, false, false)),
            (3, 1, 52 | 68) => Some(repeater("1", Facing::East, false, false)),
            (3, 1, 1) => Some(wire_state(Open, Side, Open, Side, "0")),
            (3, 1, 73) => Some(wire_state(Side, Open, Open, Side, "0")),
            (3, 1, 37) => Some(wire_state(Side, Side, Side, Side, "0")),
            (3, 1, 5 | 11 | 21 | 34) => Some(wire_state(Side, Side, Open, Side, "0")),
            (3, 1, 40 | 53 | 63 | 69) => Some(wire_state(Side, Side, Open, Side, "0")),
            (3, 0, 1..=73) => Some(chain_block()),
            (3, 1, 1..=73) => Some(wire_state(Side, Side, Open, Open, "0")),
            // z4
            (4, 0, 1 | 5 | 11 | 21 | 34 | 37 | 40 | 53 | 63 | 69 | 73) => Some(chain_block()),
            (4, 1, 1 | 5 | 69 | 73) => Some(repeater("2", Facing::South, false, false)),
            (4, 1, 11 | 21 | 53 | 63) => Some(repeater("3", Facing::South, false, false)),
            (4, 1, 34 | 37 | 40) => Some(repeater("4", Facing::South, false, false)),
            _ => None,
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(75, 2, 5)
    }
}

/// The centre source button.
fn button() -> GenericBlockState {
    let properties = HashMap::from([
        ("face".into(), "floor".into()),
        ("facing".into(), "south".into()),
        ("powered".into(), "false".into()),
    ]);
    GenericBlockState {
        name: "minecraft:stone_button".into(),
        properties,
    }
}
