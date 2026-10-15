//! Streaming litematic writer.
//!
//! Writes any declarative [`Layout`] of blocks into the litematic format as a
//! forward-only stream, so memory holds only the palette rather than the whole
//! projection. The schema is a tree of types that write themselves, so the
//! writer learns nothing of a caller beyond its bounding box and its palette.

use self::encode::Encode;
use self::nbt::{Nbt, Tag};
use crate::BlockState;
use crate::layout::{Layout, LayoutExt};
use flate2::{Compression, write::GzEncoder};
use mcdata::util::BlockPos;
use std::cell::Cell;
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const MINECRAFT_DATA_VERSION: i32 = 2975;
const VERSION: i32 = 6;
const SUB_VERSION: i32 = 1;

// Metadata
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The metadata of a written projection: its name, description, and author.
pub struct Metadata<'a> {
    /// Name of the schematic and of its single region.
    pub name: &'a str,
    /// Free-form description.
    pub description: &'a str,
    /// Author credited in the header.
    pub author: &'a str,
}

// litematic
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Writes `layout` to `sink` as a gzip-compressed litematic projection,
/// described by `metadata`.
pub(crate) fn write<L: Layout, W: Write>(
    layout: &L,
    metadata: Metadata,
    sink: W,
) -> io::Result<()> {
    let total_blocks = Cell::new(0i64);
    let schematic = Schematic::new(layout, metadata, &total_blocks)?;
    let mut nbt = Nbt::new(GzEncoder::new(sink, Compression::default()));
    schematic.write(&mut nbt)?;
    nbt.into_inner().finish().map(|_| ())
}

/// The root litematic compound: one region and the metadata that describes it.
struct Schematic<'a, L: Layout> {
    name: &'a str,
    region: RegionNbt<'a, L>,
    metadata: Header<'a>,
}

impl<'a, L: Layout> Schematic<'a, L> {
    /// Assembles the projection around a palette discovered from `layout`.
    fn new(layout: &'a L, metadata: Metadata<'a>, total_blocks: &'a Cell<i64>) -> io::Result<Self> {
        let size = layout.size();
        let palette = Palette::from_iter(layout.block_palette());
        let num_bits = palette.num_bits();
        let total = u64::from(size.volume()) * u64::from(num_bits);
        let num_longs =
            i32::try_from(total.div_ceil(64)).map_err(|_| out_of_range("BlockStates"))?;
        let now = current_time();
        Ok(Self {
            name: metadata.name,
            region: RegionNbt {
                layout,
                palette,
                size,
                num_bits,
                num_longs,
                total_blocks,
            },
            metadata: Header {
                metadata,
                enclosing_size: size,
                total_blocks,
                time_created: now,
                time_modified: now,
            },
        })
    }

    /// Writes the projection forward: the region, then the versions and metadata.
    fn write<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        out.root()?;
        out.tag(Tag::Compound, "Regions")?;
        self.region.encode(self.name, out)?;
        out.end()?; // Regions
        MINECRAFT_DATA_VERSION.encode("MinecraftDataVersion", out)?;
        VERSION.encode("Version", out)?;
        SUB_VERSION.encode("SubVersion", out)?;
        self.metadata.encode("Metadata", out)?;
        out.end() // root
    }
}

/// One region: its box, its palette, and its block states.
struct RegionNbt<'a, L: Layout> {
    layout: &'a L,
    palette: Palette,
    size: BlockPos,
    num_bits: u32,
    num_longs: i32,
    total_blocks: &'a Cell<i64>,
}

impl<L: Layout> Encode for RegionNbt<'_, L> {
    const TAG: Tag = Tag::Compound;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        BlockPos::ORIGIN.encode("Position", out)?;
        self.size.encode("Size", out)?;
        self.palette.encode("BlockStatePalette", out)?;
        let block_states = BlockStates {
            layout: self.layout,
            palette: &self.palette,
            num_bits: self.num_bits,
            num_longs: self.num_longs,
            total_blocks: self.total_blocks,
        };
        block_states.encode("BlockStates", out)?;
        out.empty_list("TileEntities")?;
        out.end()
    }
}

/// The packed block states of a region: its longs are pulled on demand.
struct BlockStates<'a, L: Layout> {
    layout: &'a L,
    palette: &'a Palette,
    num_bits: u32,
    num_longs: i32,
    total_blocks: &'a Cell<i64>,
}

impl<L: Layout> Encode for BlockStates<'_, L> {
    const TAG: Tag = Tag::LongArray;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        let packed = Packed::new(self.layout, self.palette, self.num_bits, self.total_blocks);
        out.long_array(self.num_longs, packed)
    }
}

/// The metadata compound of a projection.
struct Header<'a> {
    metadata: Metadata<'a>,
    enclosing_size: BlockPos,
    total_blocks: &'a Cell<i64>,
    time_created: i64,
    time_modified: i64,
}

impl Encode for Header<'_> {
    const TAG: Tag = Tag::Compound;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        self.metadata.name.encode("Name", out)?;
        self.metadata.author.encode("Author", out)?;
        self.metadata.description.encode("Description", out)?;
        1i32.encode("RegionCount", out)?;
        let total_volume =
            i32::try_from(self.enclosing_size.volume()).map_err(|_| out_of_range("TotalVolume"))?;
        total_volume.encode("TotalVolume", out)?;
        self.total_blocks.get().encode("TotalBlocks", out)?;
        self.time_created.encode("TimeCreated", out)?;
        self.time_modified.encode("TimeModified", out)?;
        self.enclosing_size.encode("EnclosingSize", out)?;
        out.end()
    }
}

/// Packs palette ids into little-endian 64-bit longs, one long per step, and
/// counts the blocks that are not air.
///
/// Each long is the low 64 bits of the accumulator, reinterpreted as a signed
/// `i64`: the format stores raw two's-complement longs, so neither step is a
/// numeric conversion.
struct Packed<'a, L: Layout> {
    layout: &'a L,
    palette: &'a Palette,
    size: BlockPos,
    num_bits: u32,
    index: u32,
    volume: u32,
    acc: u128,
    have: u32,
    total_blocks: &'a Cell<i64>,
}

impl<'a, L: Layout> Packed<'a, L> {
    /// Packs the ids `layout` yields through `palette`, `num_bits` apiece.
    fn new(
        layout: &'a L,
        palette: &'a Palette,
        num_bits: u32,
        total_blocks: &'a Cell<i64>,
    ) -> Self {
        let size = layout.size();
        Self {
            layout,
            palette,
            size,
            num_bits,
            index: 0,
            volume: size.volume(),
            acc: 0,
            have: 0,
            total_blocks,
        }
    }
}

impl<L: Layout> Iterator for Packed<'_, L> {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        while self.have < 64 && self.index < self.volume {
            let state = self
                .layout
                .get_block_or_air(index_to_pos(self.index, self.size));
            let id = self.palette.id_of(&state);
            self.total_blocks
                .set(self.total_blocks.get() + i64::from(id != 0));
            self.acc |= u128::try_from(id).expect("palette id fits in u128") << self.have;
            self.have += self.num_bits;
            self.index += 1;
        }
        if self.have == 0 {
            return None;
        }
        let word = self.acc as u64;
        self.acc >>= 64;
        self.have = self.have.saturating_sub(64);
        Some(word as i64)
    }
}

/// The position of index `index` in `size`, in writing order: `x` fastest, then
/// `z`, then `y`.
///
/// Sizes are non-negative, so every coordinate lies inside the box and thus
/// fits the `i32` a [`BlockPos`] is made of.
fn index_to_pos(index: u32, size: BlockPos) -> BlockPos {
    let (sx, sz) = (size.x.unsigned_abs(), size.z.unsigned_abs());
    let (x, rest) = (index % sx, index / sx);
    BlockPos::new(
        i32::try_from(x).expect("x lies inside the box"),
        i32::try_from(rest / sz).expect("y lies inside the box"),
        i32::try_from(rest % sz).expect("z lies inside the box"),
    )
}

/// Milliseconds since the Unix epoch, saturating at the `i64` limit.
fn current_time() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

// Palette
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Distinct non-air block states, ascending, one id per entry. Air is implied at
/// id 0 rather than stored, since the format always names it first; so the ids
/// stay fixed whatever order the set iterates in.
struct Palette {
    states: Vec<BlockState>,
}

impl Palette {
    /// Collects `states` into the non-air palette, ascending.
    fn from_iter<I: IntoIterator<Item = BlockState>>(states: I) -> Self {
        let air = BlockState::air();
        let mut states: Vec<BlockState> =
            states.into_iter().filter(|state| state != &air).collect();
        states.sort();
        states.dedup();
        Self { states }
    }

    /// The id of `state`, which this palette is contracted to contain. Air is id
    /// 0, and any other state is one past its place in the ascending list.
    fn id_of(&self, state: &BlockState) -> usize {
        if state == &BlockState::air() {
            return 0;
        }
        self.states
            .binary_search(state)
            .map(|index| index + 1)
            .expect("block_palette must cover every block the region emits")
    }

    /// Bits per position: the least that names every palette entry, at least two.
    fn num_bits(&self) -> u32 {
        let mut num_bits = 2;
        while (1usize << num_bits) < self.states.len() + 1 {
            num_bits += 1;
        }
        num_bits
    }
}

impl Encode for Palette {
    const TAG: Tag = Tag::List;

    /// Writes air first, then the stored states, naming a length one larger.
    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        let len = i32::try_from(self.states.len() + 1).map_err(|_| out_of_range("palette"))?;
        out.list(BlockState::TAG, len)?;
        BlockState::air().body(out)?;
        for state in &self.states {
            state.body(out)?;
        }
        Ok(())
    }
}

// Encode
//
// ++++++++++++============++++++++++++============++++++++++++============

mod encode {
    use super::{BlockPos, BlockState, Nbt, Tag};
    use std::io::{self, Write};

    /// A value that writes itself as NBT under a given name.
    pub(super) trait Encode {
        /// The tag this value is written as.
        const TAG: Tag;

        /// Writes this tag's payload, without a tag id or a name.
        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()>;

        /// Writes this value as the named tag `name`.
        fn encode<W: Write>(&self, name: &str, out: &mut Nbt<W>) -> io::Result<()> {
            out.tag(Self::TAG, name)?;
            self.body(out)
        }
    }

    impl Encode for i32 {
        const TAG: Tag = Tag::Int;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            out.int(*self)
        }
    }

    impl Encode for i64 {
        const TAG: Tag = Tag::Long;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            out.long(*self)
        }
    }

    impl Encode for str {
        const TAG: Tag = Tag::String;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            out.string(self)
        }
    }

    impl Encode for String {
        const TAG: Tag = Tag::String;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            out.string(self)
        }
    }

    impl Encode for BlockPos {
        const TAG: Tag = Tag::Compound;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            self.x.encode("x", out)?;
            self.y.encode("y", out)?;
            self.z.encode("z", out)?;
            out.end()
        }
    }

    impl Encode for BlockState {
        const TAG: Tag = Tag::Compound;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            self.name.as_ref().encode("Name", out)?;
            out.tag(Tag::Compound, "Properties")?;
            for (key, value) in &self.properties {
                value.encode(key, out)?;
            }
            out.end()?; // Properties
            out.end() // block state
        }
    }
}

// NBT
//
// ++++++++++++============++++++++++++============++++++++++++============

mod nbt {
    use super::out_of_range;
    use std::io::{self, Write};

    /// The NBT tag ids the writer emits.
    #[repr(u8)]
    #[derive(Clone, Copy)]
    pub(super) enum Tag {
        End = 0,
        Int = 3,
        Long = 4,
        String = 8,
        List = 9,
        Compound = 10,
        LongArray = 12,
    }

    /// Writes NBT bytes forward into any sink.
    ///
    /// Every tag goes straight to the sink as it is produced, so a projection can
    /// be streamed: a `TAG_Long_Array` is pulled one word at a time instead of being
    /// built whole.
    pub(super) struct Nbt<W> {
        out: W,
    }

    impl<W: Write> Nbt<W> {
        /// Wraps `out` as an NBT sink.
        pub(super) fn new(out: W) -> Self {
            Self { out }
        }

        /// Returns the sink once the projection is written.
        pub(super) fn into_inner(self) -> W {
            self.out
        }

        /// Opens the unnamed root compound.
        pub(super) fn root(&mut self) -> io::Result<()> {
            self.out.write_all(&[Tag::Compound as u8, 0, 0])
        }

        /// Writes a tag id and a length-prefixed name.
        pub(super) fn tag(&mut self, tag: Tag, name: &str) -> io::Result<()> {
            let len = u16::try_from(name.len()).map_err(|_| out_of_range("tag name"))?;
            self.out.write_all(&[tag as u8])?;
            self.out.write_all(&len.to_be_bytes())?;
            self.out.write_all(name.as_bytes())
        }

        /// Closes the compound that is open.
        pub(super) fn end(&mut self) -> io::Result<()> {
            self.out.write_all(&[Tag::End as u8])
        }

        /// Writes an `i32` payload.
        pub(super) fn int(&mut self, value: i32) -> io::Result<()> {
            self.out.write_all(&value.to_be_bytes())
        }

        /// Writes an `i64` payload.
        pub(super) fn long(&mut self, value: i64) -> io::Result<()> {
            self.out.write_all(&value.to_be_bytes())
        }

        /// Writes a length-prefixed UTF-8 payload.
        pub(super) fn string(&mut self, value: &str) -> io::Result<()> {
            let len = u16::try_from(value.len()).map_err(|_| out_of_range("string"))?;
            self.out.write_all(&len.to_be_bytes())?;
            self.out.write_all(value.as_bytes())
        }

        /// Writes a list payload header: its element tag and element count.
        pub(super) fn list(&mut self, element: Tag, len: i32) -> io::Result<()> {
            self.out.write_all(&[element as u8])?;
            self.out.write_all(&len.to_be_bytes())
        }

        /// Writes an empty named list.
        pub(super) fn empty_list(&mut self, name: &str) -> io::Result<()> {
            self.tag(Tag::List, name)?;
            self.list(Tag::End, 0)
        }

        /// Writes a `TAG_Long_Array` payload, long by long, so it is never held whole.
        pub(super) fn long_array<I: Iterator<Item = i64>>(
            &mut self,
            len: i32,
            longs: I,
        ) -> io::Result<()> {
            self.out.write_all(&len.to_be_bytes())?;
            for long in longs {
                self.out.write_all(&long.to_be_bytes())?;
            }
            Ok(())
        }
    }
}

/// The [`io::Error`] for a value that exceeds the range of its NBT field.
fn out_of_range(field: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{field} does not fit the NBT format"),
    )
}
