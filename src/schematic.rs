//! Streaming litematic writer.
//!
//! Writes any declarative [`Layout`] of blocks into the litematic format as a
//! forward-only stream, so memory holds only the palette rather than the whole
//! projection. The schema is a tree of types that write themselves, so the
//! writer learns nothing of a caller beyond its bounding box and its palette.

use self::encode::Encode;
use self::nbt::{Nbt, Tag};
use crate::{GenericBlockState, layout::Layout};
use flate2::{Compression, write::GzEncoder};
use mcdata::util::BlockPos;
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cell::Cell, collections::HashMap};

const DATA_VERSION: i32 = 2975;
const VERSION: i32 = 6;
const SUB_VERSION: i32 = 1;

// Metadata
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The header of a written projection: its name, description, and author.
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
    let solid = Cell::new(0i64);
    let schematic = Schematic::new(layout, metadata, &solid)?;
    let mut nbt = Nbt::new(GzEncoder::new(sink, Compression::default()));
    schematic.write(&mut nbt)?;
    nbt.into_inner().finish().map(|_| ())
}

/// The root litematic compound: one region and the header that describes it.
struct Schematic<'a, L: Layout> {
    name: &'a str,
    region: RegionNbt<'a, L>,
    header: Header<'a>,
}

impl<'a, L: Layout> Schematic<'a, L> {
    /// Assembles the projection around a palette discovered from `layout`.
    fn new(layout: &'a L, metadata: Metadata<'a>, solid: &'a Cell<i64>) -> io::Result<Self> {
        let size = layout.size();
        let palette = Palette::from_iter(layout.block_palette());
        let bits = palette.bits();
        let total = u64::from(size.volume()) * u64::from(bits);
        let words = i32::try_from(total.div_ceil(64)).map_err(|_| out_of_range("BlockStates"))?;
        Ok(Self {
            name: metadata.name,
            region: RegionNbt {
                layout,
                palette,
                size,
                bits,
                words,
                solid,
            },
            header: Header {
                metadata,
                size,
                solid,
                time: current_time(),
            },
        })
    }

    /// Writes the projection forward: the region, then the versions and header.
    fn write<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        out.root()?;
        out.tag(Tag::Compound, "Regions")?;
        self.region.encode(self.name, out)?;
        out.end()?; // Regions
        DATA_VERSION.encode("MinecraftDataVersion", out)?;
        VERSION.encode("Version", out)?;
        SUB_VERSION.encode("SubVersion", out)?;
        self.header.encode("Metadata", out)?;
        out.end() // root
    }
}

/// One region: its box, its palette, and its block states.
struct RegionNbt<'a, L: Layout> {
    layout: &'a L,
    palette: Palette,
    size: BlockPos,
    bits: u32,
    words: i32,
    solid: &'a Cell<i64>,
}

impl<L: Layout> Encode for RegionNbt<'_, L> {
    const TAG: Tag = Tag::Compound;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        BlockPos::ORIGIN.encode("Position", out)?;
        self.size.encode("Size", out)?;
        self.palette.blocks.encode("BlockStatePalette", out)?;
        let states = BlockStates {
            layout: self.layout,
            palette: &self.palette,
            bits: self.bits,
            words: self.words,
            solid: self.solid,
        };
        states.encode("BlockStates", out)?;
        out.empty_list("TileEntities")?;
        out.end()
    }
}

/// The packed block states of a region: its words are pulled on demand.
struct BlockStates<'a, L: Layout> {
    layout: &'a L,
    palette: &'a Palette,
    bits: u32,
    words: i32,
    solid: &'a Cell<i64>,
}

impl<L: Layout> Encode for BlockStates<'_, L> {
    const TAG: Tag = Tag::LongArray;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        let packed = Packed::new(self.layout, self.palette, self.bits, self.solid);
        out.long_array(self.words, packed)
    }
}

/// The metadata compound of a projection.
struct Header<'a> {
    metadata: Metadata<'a>,
    size: BlockPos,
    solid: &'a Cell<i64>,
    time: i64,
}

impl Encode for Header<'_> {
    const TAG: Tag = Tag::Compound;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        self.metadata.name.encode("Name", out)?;
        self.metadata.author.encode("Author", out)?;
        self.metadata.description.encode("Description", out)?;
        1i32.encode("RegionCount", out)?;
        let volume = i32::try_from(self.size.volume()).map_err(|_| out_of_range("TotalVolume"))?;
        volume.encode("TotalVolume", out)?;
        self.solid.get().encode("TotalBlocks", out)?;
        self.time.encode("TimeCreated", out)?;
        self.time.encode("TimeModified", out)?;
        self.size.encode("EnclosingSize", out)?;
        out.end()
    }
}

/// Packs palette ids into little-endian 64-bit words, one word per step, and
/// counts the cells that are not air.
///
/// Each word is the low 64 bits of the accumulator, reinterpreted as a signed
/// `i64`: the format stores raw two's-complement longs, so neither step is a
/// numeric conversion.
struct Packed<'a, L: Layout> {
    layout: &'a L,
    palette: &'a Palette,
    size: BlockPos,
    bits: u32,
    cell: u32,
    cells: u32,
    acc: u128,
    have: u32,
    solid: &'a Cell<i64>,
}

impl<'a, L: Layout> Packed<'a, L> {
    /// Packs the ids `layout` yields through `palette`, `bits` apiece.
    fn new(layout: &'a L, palette: &'a Palette, bits: u32, solid: &'a Cell<i64>) -> Self {
        let size = layout.size();
        Self {
            layout,
            palette,
            size,
            bits,
            cell: 0,
            cells: size.volume(),
            acc: 0,
            have: 0,
            solid,
        }
    }
}

impl<L: Layout> Iterator for Packed<'_, L> {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        while self.have < 64 && self.cell < self.cells {
            let block = self.layout.get_block_or_air(cell_pos(self.cell, self.size));
            let id = self.palette.id_of(&block);
            self.solid.set(self.solid.get() + i64::from(id != 0));
            self.acc |= u128::try_from(id).expect("palette id fits in u128") << self.have;
            self.have += self.bits;
            self.cell += 1;
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

/// The position of the `index`-th cell of `size`, in writing order: `x` fastest,
/// then `z`, then `y`.
///
/// Sizes are non-negative, so every coordinate lies inside the box and thus
/// fits the `i32` a [`BlockPos`] is made of.
fn cell_pos(index: u32, size: BlockPos) -> BlockPos {
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

/// Distinct block states in first-seen order, with a map from state to id so a
/// cell resolves without a scan.
struct Palette {
    blocks: Vec<GenericBlockState>,
    ids: HashMap<GenericBlockState, usize>,
}

impl Palette {
    /// Collects `blocks` into a palette ordered canonically: air first, then the
    /// rest ascending, so the ids stay fixed whatever order the set iterates in.
    fn from_iter<I: IntoIterator<Item = GenericBlockState>>(blocks: I) -> Self {
        let air = GenericBlockState::air();
        let mut blocks: Vec<GenericBlockState> =
            blocks.into_iter().filter(|block| block != &air).collect();
        blocks.sort();
        blocks.dedup();
        blocks.insert(0, air);
        let ids: HashMap<GenericBlockState, usize> = blocks
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, block)| (block, id))
            .collect();
        Self { blocks, ids }
    }

    /// The id of `block`, which this palette is contracted to contain.
    fn id_of(&self, block: &GenericBlockState) -> usize {
        self.ids
            .get(block)
            .copied()
            .expect("block_palette must cover every block the region emits")
    }

    /// Bits per cell: the least that still names every palette entry, at least two.
    fn bits(&self) -> u32 {
        let mut bits = 2;
        while (1usize << bits) < self.blocks.len() {
            bits += 1;
        }
        bits
    }
}

// Encode
//
// ++++++++++++============++++++++++++============++++++++++++============

mod encode {
    use super::{BlockPos, GenericBlockState, Nbt, Tag, out_of_range};
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

    impl Encode for GenericBlockState {
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

    impl<T: Encode> Encode for Vec<T> {
        const TAG: Tag = Tag::List;

        fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
            let len = i32::try_from(self.len()).map_err(|_| out_of_range("list length"))?;
            out.list(T::TAG, len)?;
            for item in self {
                item.body(out)?;
            }
            Ok(())
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

        /// Writes a `TAG_Long_Array` payload, word by word, so it is never held whole.
        pub(super) fn long_array<I: Iterator<Item = i64>>(
            &mut self,
            len: i32,
            words: I,
        ) -> io::Result<()> {
            self.out.write_all(&len.to_be_bytes())?;
            for word in words {
                self.out.write_all(&word.to_be_bytes())?;
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
