use super::{AsLayout, Layout};
use rsnbs::note::Note;
use rsnbs::types::TimeAnchor;

pub struct TorrentLayout;

impl TorrentLayout {
    pub fn new<Trk, A, T>(notes: Trk) -> Self
    where
        Trk: IntoIterator<Item = (A, T)>,
        A: TimeAnchor,
        T: Into<Note>,
    {
        let _ = notes;
        todo!()
    }
}

impl AsLayout for TorrentLayout {
    fn as_layout(&self) -> &impl Layout {
        &()
    }
}

#[allow(dead_code)]
struct Template;

#[allow(dead_code)]
impl Template {
    /// The wall as a picture: one row per layer from the top (y = 70) down to the
    /// bottom (y = 2), one character per grid step from the left across, `#` a
    /// track centre and `.` an empty cell. The grid spans x = 1, 5, …, 97.
    const WALL: [&str; 18] = [
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

    /// The wall's pivot: the central track on the source row, as the listener's
    /// place in the wall's own plane.
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

    /// Reads `WALL` back onto the wall's four-step grid. Every cell writes the
    /// next slot and only a mark advances the cursor, so an empty cell is simply
    /// overwritten by the mark that follows it.
    const fn tracks() -> [(i32, i32); 176] {
        let mut tracks = [(0, 0); 176];
        let (rows, cols) = (Self::WALL.len(), Self::WALL[0].len());
        let (mut n, mut k) = (0, 0);
        while k < rows * cols && n < 176 {
            let (r, x) = (k / cols, k % cols);
            tracks[n] = (x as i32 * 4 + 1, (rows - 1 - r) as i32 * 4 + 2);
            n += (Self::WALL[r].as_bytes()[x] == b'#') as usize;
            k += 1;
        }
        tracks
    }

    fn new<I: IntoIterator<Item = Note>>(notes: I) -> Self {
        let _ = notes;
        todo!()
    }
}
