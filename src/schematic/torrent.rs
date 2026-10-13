use super::{AsLayout, EvenlyArranged, Facing, Layout, Overlaid, WireConn};
use super::{air, chain_block, inst_block, note_block, repeater, wire_state};
use crate::playable;
use mcdata::{GenericBlockState, util::BlockPos};
use rsnbs::note::{Note, Tone};
use rsnbs::types::TimeAnchor;
use rustmatica::Litematic;
use std::collections::{BTreeMap, HashMap};
use std::iter;

// Layout: TorrentLayout
//
// A torrent reads as a tunnel built from a series of slices rather than a
// single row: every tick lays a whole slice, and silent ticks keep their slice
// so the tunnel stays evenly spaced. The tunnel is read from one fixed
// viewpoint, so each voice is placed by where it is heard. The whole torrent
// fires from one fixed startup.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A note stream projected as a torrent: one slice per tick off a fixed startup.
pub struct TorrentLayout(Overlaid<Linkage, EvenlyArranged<Template>>);

impl TorrentLayout {
    /// Projects a note stream, one slice per redstone tick.
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
        let mut slices: Vec<Template> = Vec::new();
        while let Some(&(next, _)) = notes.peek() {
            let len = (next as usize).max(slices.len());
            slices.resize_with(len, || Template::new(iter::empty()));
            let slice = iter::from_fn(|| notes.next_if(|&(t, _)| t == next).map(|(_, note)| note));
            slices.push(Template::new(slice));
        }
        if slices.is_empty() {
            slices.push(Template::new(iter::empty()));
        }

        let linkage = Linkage::new();
        let anchor = BlockPos::new(0, 0, linkage.size().z - 1);
        let slices = EvenlyArranged::new(slices, BlockPos::new(0, 0, 2));
        let size = BlockPos::new(slices.size().x, slices.size().y, anchor.z + slices.size().z);
        let layout = Overlaid::new(BlockPos::ORIGIN, linkage, anchor, slices, size);
        Self(layout)
    }
}

impl AsLayout for TorrentLayout {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Templates: Template & Linkage
//
// Linkage is the fixed front that sets every track running together; Template
// is one slice of the tunnel. Every slot is heard at a panning and volume from
// the axis, so a voice lands on the slot that hears it nearest, and each slot
// holds only one voice.
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The fixed startup that sets every track running together: the distribution
/// network and its per-track risers, confirmed on a real build and embedded here
/// whole as a projection, read back block by block.
struct Linkage(Litematic<GenericBlockState>);

impl Linkage {
    /// Loads the built-in startup from the projection embedded beside this file.
    fn new() -> Self {
        const BYTES: &[u8] = include_bytes!("torrent_linkage.litematic");
        let litematic = Litematic::from_bytes(BYTES).expect("built-in linkage is valid");
        Self(litematic)
    }
}

impl Layout for Linkage {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        Some(self.0.regions[0].get_block(pos).clone())
    }

    fn size(&self) -> BlockPos {
        self.0.enclosing_size()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template {
    slots: BTreeMap<(i32, i32), Tone>,
}

#[allow(dead_code)]
impl Template {
    /// The slice as a picture: one row per layer from the top (y = 70) down to the
    /// bottom (y = 2), one character per grid step from the left across, `#` a
    /// track centre and `.` an empty cell. The grid spans x = 1, 5, …, 97.
    const SLICE: [&str; 18] = [
        "..#.........#.........#..",
        ".##.......#####.......##.",
        ".###...##...#...##...###.",
        "..###.####.....####.###..",
        "#...####.........####...#",
        "###...................###",
        "########.........########",
        "####....#...#...#....####",
        "......#############......",
        "....####...###...####....",
        ".######..#..#..#..######.",
        ".####....#..#..#....####.",
        "..##....##..#..##....##..",
        "...#....##..#..##....#...",
        ".......##..###..##.......",
        ".......##..###..##.......",
        "......###..###..###......",
        "........#.#####.#........",
    ];

    /// The track centres, top row first and left to right within a row.
    const TRACKS: [(i32, i32); 176] = Self::tracks();

    /// The slice's pivot: the central track on the source row, as the listener's
    /// place in the slice's own plane.
    const AXIS: (i32, i32) = (49, 46);

    /// Each slot: the panning and volume it is heard at from `AXIS`, with its own
    /// position as the note block's place.
    const SLOTS: [((i8, u8), (i32, i32)); 528] = Self::slots();

    /// Listens to the whole slice from four blocks straight out in front of `AXIS`,
    /// reading it back as a set of slot positions keyed by the panning and volume each
    /// is heard at. Every track gives three slots, at its own x and the cells either
    /// side.
    ///
    /// A track's volume first falls off linearly with its distance from the axis, but
    /// that linear amplitude reads as too sharp a drop at the far side, so a sensory
    /// conversion lifts it by a root to give the even falloff the senses expect.
    ///
    /// A track's panning is read from the horizontal angle it makes with the
    /// listener, leaning left as it gains x.
    const fn slots() -> [((i8, u8), (i32, i32)); 528] {
        let (cx, cy) = Self::AXIS;
        let mut slots = [((0, 0), (0, 0)); 528];
        let mut k = 0;
        while k < slots.len() {
            let track = Self::TRACKS[k / 3];
            let place = (track.0 + [0, -1, 1][k % 3], track.1);
            let (dx, dy) = (place.0 - cx, place.1 - cy);
            let d = (dx * dx + dy * dy).isqrt();
            let lin = if d > 48 { 0 } else { 48 - d };
            let volume = (lin * 10000 / 48).isqrt();
            let panning = -dx * 100 / (dx * dx + 16).isqrt();
            slots[k] = ((panning as i8, volume as u8), place);
            k += 1;
        }
        slots
    }

    /// Reads `SLICE` back onto the slice's four-step grid. Every cell writes the
    /// next slot and only a mark advances the cursor, so an empty cell is simply
    /// overwritten by the mark that follows it.
    const fn tracks() -> [(i32, i32); 176] {
        let mut tracks = [(0, 0); 176];
        let (rows, cols) = (Self::SLICE.len(), Self::SLICE[0].len());
        let (mut n, mut k) = (0, 0);
        while k < rows * cols && n < 176 {
            let (r, x) = (k / cols, k % cols);
            tracks[n] = (x as i32 * 4 + 1, (rows - 1 - r) as i32 * 4 + 2);
            n += (Self::SLICE[r].as_bytes()[x] == b'#') as usize;
            k += 1;
        }
        tracks
    }

    /// Sounds this slice's voices. A voice lands on the slot that hears it most
    /// alike, and every slot holds only one voice, so a voice with no free slot
    /// left goes silent rather than displacing one already placed.
    fn new<I: IntoIterator<Item = Note>>(notes: I) -> Self {
        let mut free = Vec::from(Self::SLOTS);
        let mut slots = BTreeMap::new();
        for note in playable(notes) {
            let heard = (note.panning.get(), note.velocity.get());
            let near = free.iter().enumerate();
            let pick = near.min_by_key(|(_, (at, _))| Self::score(heard, *at));
            let chosen = pick.map(|(i, &(_, place))| (i, place));
            let Some((i, place)) = chosen else { continue };
            free.swap_remove(i);
            slots.insert(place, note.tone);
        }
        Self { slots }
    }

    /// Scores how alike two heard positions are, so a voice can be matched to the
    /// slot that hears it nearest: zero when they coincide and larger as they
    /// drift apart in panning and volume. Panning weighs more than volume, though
    /// the exact balance is not pinned down.
    ///
    /// Panning and volume carry no settled meaning yet, so this may move onto a
    /// dedicated type once one exists; for now the pair stays a plain tuple.
    fn score(a: (i8, u8), b: (i8, u8)) -> i32 {
        let dp = i32::from(a.0) - i32::from(b.0);
        let dv = i32::from(a.1) - i32::from(b.1);
        dp * dp + dv * dv
    }
}

impl Layout for Template {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        let Some(local @ (local_x, local_y)) = Self::TRACKS.into_iter().find_map(|(x, y)| {
            let (x, y) = (x - pos.x, y - pos.y);
            ((-1..=1).contains(&x) && (0..=2).contains(&y)).then_some((x, y))
        }) else {
            return None;
        };

        // z = 0
        if pos.z == 0 {
            let repeater = || repeater("1", Facing::South, false, false);
            let drive = (local == (0, 0)).then_some(repeater());
            let scaffold = matches!(local, (0, 1 | 2)).then_some(barrier());
            return drive.or(scaffold);
        }
        debug_assert!(pos.z == 1);

        // attempt to hit
        if let Some(tone) = self.slots.get(&(pos.x, pos.y)) {
            return Some(note_block(Some(tone), air));
        } else if let Some(tone) = self.slots.get(&(pos.x, pos.y + 1)) {
            return Some(inst_block(Some(tone), air));
        } else if self.slots.contains_key(&(pos.x, pos.y + 2)) {
            return Some(barrier());
        } else if local_x != 0 {
            return None;
        }
        debug_assert!(local_x == 0);

        // main track
        if local_y != 0 {
            return Some(barrier());
        }
        debug_assert!(local == (0, 0));

        // Provide circuit support for note blocks on the sides
        use WireConn::{None as Open, Side};
        let west = self.slots.contains_key(&(pos.x - 1, pos.y));
        let east = self.slots.contains_key(&(pos.x + 1, pos.y));
        match west || east {
            true => Some(chain_block()),
            false => Some(wire_state(Open, Open, Side, Side, "0")),
        }
    }

    fn size(&self) -> BlockPos {
        BlockPos::new(99, 71, 2)
    }
}

/// The barrier that scaffolds a track.
#[allow(dead_code)]
fn barrier() -> GenericBlockState {
    GenericBlockState {
        name: "minecraft:barrier".into(),
        properties: HashMap::from([("waterlogged".into(), "false".into())]),
    }
}
