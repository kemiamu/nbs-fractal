//! Torrent layout for NBS song projection.

use super::blocks::{Facing, WireConn, chain_block, inst_block, note_block, repeater, wire_state};
use super::{AsLayout, EvenlyArranged, Layout, Overlaid, air, include};
use crate::playable;
use mcdata::{GenericBlockState, util::BlockPos};
use rsnbs::note::{Note, Tone};
use rsnbs::types::TimeAnchor;
use rustmatica::Litematic;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

/// A tunnel of slices with pan and volume: whole slices laid tick by tick, read
/// from one fixed viewpoint, so a voice lands where it is heard.
pub struct TorrentLayout<'a>(Overlaid<Linkage, EvenlyArranged<Template<'a>>>);

impl<'a> TorrentLayout<'a> {
    /// The slice as a picture: one row per layer from the top down, one character per
    /// grid step from the left across, `#` a track centre and `.` an empty cell. The
    /// grid spans x = 1, 5, …, 97.
    const SLICE: [&'static str; 18] = [
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

    /// The slice's pivot: the central track, as the listener's place in the slice's
    /// own plane.
    const AXIS: (i32, i32) = (49, 46);

    /// Places a track's axis-relative centre back into the slice's own plane, where
    /// east is negative x and up is positive y, so the origin maps to `AXIS`.
    fn absolute(track: (i32, i32)) -> (i32, i32) {
        (Self::AXIS.0 - track.0, Self::AXIS.1 + track.1)
    }

    /// The built-in track centres, read off `SLICE` and held relative to `AXIS`: east
    /// is negative x and up is positive y, so every track sits on the four-step grid
    /// around the origin.
    pub const DEFAULT_TRACKS: [(i32, i32); 176] = Self::tracks();

    /// Reads `SLICE` back onto the slice's four-step grid, relative to `AXIS`. Every
    /// cell writes the next track and only a mark advances the cursor, so an empty
    /// cell is simply overwritten by the mark that follows it.
    const fn tracks() -> [(i32, i32); 176] {
        let mut tracks = [(0, 0); 176];
        let (rows, cols) = (Self::SLICE.len(), Self::SLICE[0].len());
        let (mut n, mut k) = (0, 0);
        while k < rows * cols && n < 176 {
            let (r, x) = (k / cols, k % cols);
            let (tx, ty) = (x as i32 * 4 + 1, (rows - 1 - r) as i32 * 4 + 2);
            tracks[n] = (Self::AXIS.0 - tx, ty - Self::AXIS.1);
            n += (Self::SLICE[r].as_bytes()[x] == b'#') as usize;
            k += 1;
        }
        tracks
    }

    /// Builds the tunnel over filled data: the fixed startup in front, the slices
    /// stepped evenly two deep behind it.
    pub fn new(tracks: &'a [(i32, i32)], slices: Vec<BTreeMap<(i32, i32, i32), Tone>>) -> Self {
        debug_assert!(tracks.is_sorted(), "tracks must be sorted for lookup");
        let linkage = Linkage::new();
        let anchor = BlockPos::new(0, 0, linkage.size().z - 1);
        let slices = slices.into_iter().map(|slots| Template { tracks, slots });
        let slices = EvenlyArranged::new(slices, BlockPos::new(0, 0, 2));
        let span = include(linkage.size(), slices.size());
        let size = BlockPos::new(span.x, span.y, anchor.z + slices.size().z);
        let layout = Overlaid::new(BlockPos::ORIGIN, linkage, anchor, slices, size);
        Self(layout)
    }

    /// Fills the effective tracks and one voice map per tick, routing every voice
    /// to the free slot that hears it nearest. Tracks are axis-relative, as
    /// `DEFAULT_TRACKS` and `--track` give them, and come back sorted so the
    /// layout can look them up by key. With `omit_empty`, a track that never
    /// sounds is dropped.
    pub fn fill<Trk, A, T, Tracks>(
        notes: Trk,
        tracks: Tracks,
        omit_empty: bool,
    ) -> (Vec<(i32, i32)>, Vec<BTreeMap<(i32, i32, i32), Tone>>)
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Note>,
        Tracks: IntoIterator<Item = (i32, i32)>,
    {
        let mut tracks: Vec<(i32, i32)> = tracks.into_iter().map(Self::absolute).collect();
        let slots = Self::slots(&tracks);
        let mut notes = notes
            .into_iter()
            .map(|(anchor, note)| (anchor.into_tick(), note.into()))
            .peekable();
        let mut slices = Vec::new();
        while let Some(&(next, _)) = notes.peek() {
            slices.resize_with(slices.len().max(next as usize), Default::default);
            let voices = iter::from_fn(|| notes.next_if(|&(t, _)| t == next).map(|(_, note)| note));
            slices.push(Self::place(voices, &slots));
        }
        slices.resize_with(slices.len().max(1), Default::default);

        if omit_empty {
            let keys = slices.iter().flat_map(|slice| slice.keys());
            let used: BTreeSet<(i32, i32)> = keys.map(|&(x, y, _)| (x, y)).collect();
            tracks.retain(|track| used.contains(track));
        }
        // Slots keep their input order, but the layout looks tracks up by key.
        tracks.sort_unstable();
        (tracks, slices)
    }

    /// Sounds one tick's voices, each on the free slot that hears it nearest. A
    /// voice the slice cannot sound is skipped, so it never crowds out a voice
    /// that does; a voice with no free slot left goes silent.
    fn place<I: IntoIterator<Item = Note>>(
        notes: I,
        slots: &[((i8, u8), (i32, i32, i32))],
    ) -> BTreeMap<(i32, i32, i32), Tone> {
        let mut free = slots.to_vec();
        let mut placed = BTreeMap::new();
        for note in playable(notes) {
            let heard = (note.panning.get(), note.velocity.get());
            let near = free.iter().enumerate();
            let pick = near.min_by_key(|(_, (at, _))| Self::score(heard, *at));
            let Some(hit) = pick else { break };
            let (i, &(_, place)) = hit;
            free.swap_remove(i);
            placed.insert(place, note.tone);
        }
        placed
    }

    /// Listens to the whole slice from four blocks straight out in front of
    /// `AXIS`, reading it back as slots keyed by the panning and volume each is
    /// heard at. Every track gives three slots, at its own x and the cells either
    /// side, each keyed by `(track_x, track_y, local_x)`.
    ///
    /// A track's volume first falls off linearly with its distance from the axis,
    /// but that linear amplitude reads as too sharp a drop at the far side, so a
    /// sensory conversion lifts it by a root to give the even falloff the senses
    /// expect.
    ///
    /// A track's panning is read from the horizontal angle it makes with the
    /// listener, leaning left as it gains x.
    fn slots(tracks: &[(i32, i32)]) -> Vec<((i8, u8), (i32, i32, i32))> {
        let (cx, cy) = Self::AXIS;
        let mut slots = Vec::with_capacity(tracks.len() * 3);
        let cells = tracks.iter().flat_map(|&t| [0, -1, 1].map(move |c| (t, c)));
        for ((tx, ty), cell) in cells {
            let (dx, dy) = (tx + cell - cx, ty - cy);
            let d = (dx * dx + dy * dy).isqrt();
            let lin = if d > 48 { 0 } else { 48 - d };
            let volume = (lin * 10000 / 48).isqrt();
            let panning = -dx * 100 / (dx * dx + 16).isqrt();
            slots.push(((panning as i8, volume as u8), (tx, ty, cell)));
        }
        slots
    }

    /// Scores how alike two heard positions are, so a voice can be matched to the
    /// slot that hears it nearest: zero when they coincide and larger as they
    /// drift apart in panning and volume. Panning covers twice volume's range, so
    /// the volume term is doubled, leaving panning twice as heavy overall.
    ///
    /// Panning and volume carry no settled meaning yet, so this may move onto a
    /// dedicated type once one exists; for now the pair stays a plain tuple.
    fn score(a: (i8, u8), b: (i8, u8)) -> i32 {
        let dp = i32::from(a.0) - i32::from(b.0);
        let dv = i32::from(a.1) - i32::from(b.1);
        dp * dp + dv * dv * 2
    }
}

impl AsLayout for TorrentLayout<'_> {
    fn as_layout(&self) -> &impl Layout {
        &self.0
    }
}

// Templates: Template & Linkage
//
// Linkage is the fixed front that sets every track running together; Template
// is one slice of the tunnel, rendering the voices the reel filed for its tick.
// Every slot is heard at a panning and volume, so a voice lands on the slot
// that hears it nearest.
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

/// One slice of the tunnel: the effective tracks it renders against, and the
/// voices it carries, keyed by `(track_x, track_y, local_x)`.
struct Template<'a> {
    /// Track centres sorted by `(x, y)`, so `block_at` can binary-search them.
    tracks: &'a [(i32, i32)],
    slots: BTreeMap<(i32, i32, i32), Tone>,
}

impl Layout for Template<'_> {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
        // The track holding this position, by binary search: within `±1` in x and
        // `[-2, 0]` in y, and centres four apart, so one x column is the only one.
        let lo = self.tracks.partition_point(|&(tx, _)| tx < pos.x - 1);
        let hi = self.tracks.partition_point(|&(tx, _)| tx <= pos.x + 1);
        let col = &self.tracks[lo..hi];
        let at = col.partition_point(|&(_, ty)| ty < pos.y);
        let near = col[at..].first().filter(|&&(_, ty)| ty <= pos.y + 2);
        let Some(&(tx, ty)) = near else { return None };
        let (lx, ly) = (pos.x - tx, pos.y - ty);

        // z = 0: the drive row, one repeater on each track's centre.
        if pos.z == 0 {
            let repeater = || repeater("1", Facing::South, false, false);
            let drive = (lx == 0 && ly == 0).then_some(repeater());
            let scaffold = (lx == 0 && ly != 0).then_some(barrier());
            return drive.or(scaffold);
        }
        debug_assert!(pos.z == 1);

        // z = 1: the note row. The slot key names the track cell, and the layer
        // the position sits on picks the block.
        let voice = self.slots.get(&(tx, ty, lx));
        match (voice, ly) {
            (Some(tone), 0) => return Some(note_block(Some(tone), air)),
            (Some(tone), -1) => return Some(inst_block(Some(tone), air)),
            (Some(_), -2) => return Some(barrier()),
            _ => { /* nothing */ }
        }
        if lx != 0 {
            return None;
        }
        if ly != 0 {
            return Some(barrier());
        }

        // Provide circuit support for note blocks on the sides
        use WireConn::{None as Open, Side};
        let west = self.slots.contains_key(&(tx, ty, -1));
        let east = self.slots.contains_key(&(tx, ty, 1));
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
fn barrier() -> GenericBlockState {
    GenericBlockState {
        name: "minecraft:barrier".into(),
        properties: HashMap::from([("waterlogged".into(), "false".into())]),
    }
}
