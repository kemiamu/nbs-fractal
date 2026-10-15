//! Streaming litematic writer.
//!
//! Writes any declarative [`Region`] of blocks into the litematic format as a
//! forward-only stream, so memory holds only the palette rather than the whole
//! projection. The schema is a tree of types that write themselves, so the
//! writer learns nothing of a caller beyond its bounding box and its palette.

use flate2::{Compression, write::GzEncoder};
use itertools::iproduct;
use mcdata::{BlockState, GenericBlockState, util::BlockPos};
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cell::Cell, collections::HashMap, iter};

const DATA_VERSION: i32 = 2975;
const VERSION: i32 = 6;
const SUB_VERSION: i32 = 1;

// Region & Metadata
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A declarative block source: a bounding box and the block at each cell.
///
/// This is the whole interface the writer needs: give it a size and a block per
/// cell and it writes a projection, with no knowledge of how they were arranged.
pub trait Region {
    /// Bounding box size, in blocks.
    fn size(&self) -> BlockPos;

    /// Block at the given in-bounds position; `None` is air.
    fn get_block(&self, pos: BlockPos) -> Option<GenericBlockState>;

    /// The palette the region draws on: every distinct block it can emit, air
    /// first, in first-seen writing order.
    ///
    /// A region that already knows its palette declares it here to spare the
    /// writer a discovery pass; the default walks [`get_block`](Self::get_block)
    /// over the whole box.
    fn block_palette(&self) -> impl Iterator<Item = GenericBlockState> + '_ {
        let size = self.size();
        let cell = |(y, z, x)| {
            let block = self.get_block(BlockPos::new(x, y, z));
            block.unwrap_or_else(GenericBlockState::air)
        };
        let cells = iproduct!(0..size.y, 0..size.z, 0..size.x).map(cell);
        let declared = iter::once(GenericBlockState::air()).chain(cells);
        Palette::from_iter(declared).blocks.into_iter()
    }
}

/// The header of a written projection: its name, description, and author.
pub struct Metadata<'a> {
    /// Name of the schematic and of its single region.
    pub name: &'a str,
    /// Free-form description.
    pub description: &'a str,
    /// Author credited in the header.
    pub author: &'a str,
}

// Encode
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A value that writes itself as NBT under a given name.
trait Encode {
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

// litematic
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Writes `region` to `sink` as a gzip-compressed litematic projection.
pub fn save<R: Region, W: Write>(region: &R, metadata: Metadata, sink: W) -> io::Result<()> {
    let solid = Cell::new(0i64);
    let schematic = Schematic::new(region, metadata, &solid)?;
    let mut nbt = Nbt::new(GzEncoder::new(sink, Compression::default()));
    schematic.write(&mut nbt)?;
    nbt.into_inner().finish()?;
    Ok(())
}

/// The root litematic compound: one region and the header that describes it.
struct Schematic<'a, R: Region> {
    name: &'a str,
    region: RegionNbt<'a, R>,
    header: Header<'a>,
}

impl<'a, R: Region> Schematic<'a, R> {
    /// Assembles the projection around a palette discovered from `region`.
    fn new(region: &'a R, metadata: Metadata<'a>, solid: &'a Cell<i64>) -> io::Result<Self> {
        let size = region.size();
        let palette = Palette::from_iter(region.block_palette());
        let bits = palette.bits();
        let total = u64::from(size.volume()) * u64::from(bits);
        let words = i32::try_from(total.div_ceil(64)).map_err(|_| out_of_range("BlockStates"))?;
        Ok(Self {
            name: metadata.name,
            region: RegionNbt {
                region,
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
struct RegionNbt<'a, R: Region> {
    region: &'a R,
    palette: Palette,
    size: BlockPos,
    bits: u32,
    words: i32,
    solid: &'a Cell<i64>,
}

impl<R: Region> Encode for RegionNbt<'_, R> {
    const TAG: Tag = Tag::Compound;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        BlockPos::ORIGIN.encode("Position", out)?;
        self.size.encode("Size", out)?;
        self.palette.blocks.encode("BlockStatePalette", out)?;
        let states = BlockStates {
            region: self.region,
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
struct BlockStates<'a, R: Region> {
    region: &'a R,
    palette: &'a Palette,
    bits: u32,
    words: i32,
    solid: &'a Cell<i64>,
}

impl<R: Region> Encode for BlockStates<'_, R> {
    const TAG: Tag = Tag::LongArray;

    fn body<W: Write>(&self, out: &mut Nbt<W>) -> io::Result<()> {
        let packed = Packed::new(self.region, self.palette, self.bits, self.solid);
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
struct Packed<'a, R: Region> {
    region: &'a R,
    palette: &'a Palette,
    size: BlockPos,
    bits: u32,
    cell: u32,
    cells: u32,
    acc: u128,
    have: u32,
    solid: &'a Cell<i64>,
}

impl<'a, R: Region> Packed<'a, R> {
    /// Packs the ids `region` yields through `palette`, `bits` apiece.
    fn new(region: &'a R, palette: &'a Palette, bits: u32, solid: &'a Cell<i64>) -> Self {
        let size = region.size();
        Self {
            region,
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

impl<R: Region> Iterator for Packed<'_, R> {
    type Item = i64;

    fn next(&mut self) -> Option<i64> {
        while self.have < 64 && self.cell < self.cells {
            let block = self
                .region
                .get_block(cell_pos(self.cell, self.size))
                .unwrap_or_else(GenericBlockState::air);
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

/// Distinct block states in first-seen order, with a hashed lookup so a cell
/// resolves in constant time rather than a linear scan.
struct Palette {
    blocks: Vec<GenericBlockState>,
    buckets: HashMap<u64, Vec<usize>, BuildHasherDefault<Identity>>,
}

impl Palette {
    /// Collects `blocks` into a palette, keeping the first occurrence of each.
    fn from_iter<I: IntoIterator<Item = GenericBlockState>>(blocks: I) -> Self {
        let mut palette = Self {
            blocks: Vec::new(),
            buckets: HashMap::default(),
        };
        for block in blocks {
            palette.insert(block);
        }
        palette
    }

    /// Adds `block` if it is new, keeping the id already assigned otherwise.
    fn insert(&mut self, block: GenericBlockState) {
        let hash = block_hash(&block);
        if self.find(hash, &block).is_some() {
            return;
        }
        let id = self.blocks.len();
        self.buckets.entry(hash).or_default().push(id);
        self.blocks.push(block);
    }

    /// The id of `block`, which this palette is contracted to contain.
    fn id_of(&self, block: &GenericBlockState) -> usize {
        self.find(block_hash(block), block)
            .expect("block_palette must cover every block the region emits")
    }

    /// The id of `block` if present, looked up by its precomputed `hash`.
    fn find(&self, hash: u64, block: &GenericBlockState) -> Option<usize> {
        let ids = self.buckets.get(&hash)?;
        ids.iter().copied().find(|&id| &self.blocks[id] == block)
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

/// A pass-through hasher for keys that are already one flat `u64`.
#[derive(Default)]
struct Identity(u64);

impl Hasher for Identity {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _bytes: &[u8]) {
        unreachable!("only u64 keys are hashed")
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

/// Hashes a block state with FNV-1a, combining its properties commutatively so
/// their unordered map still hashes to one value.
fn block_hash(block: &GenericBlockState) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in block.name.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(PRIME);
    }
    let mut properties = 0u64;
    for (key, value) in &block.properties {
        let mut pair = OFFSET;
        for byte in key.as_bytes().iter().chain(value.as_bytes()) {
            pair = (pair ^ u64::from(*byte)).wrapping_mul(PRIME);
        }
        properties = properties.wrapping_add(pair);
    }
    hash ^ properties
}

// NBT
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The NBT tag ids the writer emits.
#[repr(u8)]
#[derive(Clone, Copy)]
enum Tag {
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
struct Nbt<W> {
    out: W,
}

impl<W: Write> Nbt<W> {
    /// Wraps `out` as an NBT sink.
    fn new(out: W) -> Self {
        Self { out }
    }

    /// Returns the sink once the projection is written.
    fn into_inner(self) -> W {
        self.out
    }

    /// Opens the unnamed root compound.
    fn root(&mut self) -> io::Result<()> {
        self.out.write_all(&[Tag::Compound as u8, 0, 0])
    }

    /// Writes a tag id and a length-prefixed name.
    fn tag(&mut self, tag: Tag, name: &str) -> io::Result<()> {
        let len = u16::try_from(name.len()).map_err(|_| out_of_range("tag name"))?;
        self.out.write_all(&[tag as u8])?;
        self.out.write_all(&len.to_be_bytes())?;
        self.out.write_all(name.as_bytes())
    }

    /// Closes the compound that is open.
    fn end(&mut self) -> io::Result<()> {
        self.out.write_all(&[Tag::End as u8])
    }

    /// Writes an `i32` payload.
    fn int(&mut self, value: i32) -> io::Result<()> {
        self.out.write_all(&value.to_be_bytes())
    }

    /// Writes an `i64` payload.
    fn long(&mut self, value: i64) -> io::Result<()> {
        self.out.write_all(&value.to_be_bytes())
    }

    /// Writes a length-prefixed UTF-8 payload.
    fn string(&mut self, value: &str) -> io::Result<()> {
        let len = u16::try_from(value.len()).map_err(|_| out_of_range("string"))?;
        self.out.write_all(&len.to_be_bytes())?;
        self.out.write_all(value.as_bytes())
    }

    /// Writes a list payload header: its element tag and element count.
    fn list(&mut self, element: Tag, len: i32) -> io::Result<()> {
        self.out.write_all(&[element as u8])?;
        self.out.write_all(&len.to_be_bytes())
    }

    /// Writes an empty named list.
    fn empty_list(&mut self, name: &str) -> io::Result<()> {
        self.tag(Tag::List, name)?;
        self.list(Tag::End, 0)
    }

    /// Writes a `TAG_Long_Array` payload, word by word, so it is never held whole.
    fn long_array<I: Iterator<Item = i64>>(&mut self, len: i32, words: I) -> io::Result<()> {
        self.out.write_all(&len.to_be_bytes())?;
        for word in words {
            self.out.write_all(&word.to_be_bytes())?;
        }
        Ok(())
    }
}

/// The [`io::Error`] for a value that exceeds the range of its NBT field.
fn out_of_range(field: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{field} does not fit the NBT format"),
    )
}
