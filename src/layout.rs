//! Projection layouts: the note-to-block arrangement a song projects into.

use crate::BlockState;
use crate::schematic::{self, Metadata};
use itertools::iproduct;
use mcdata::util::BlockPos;
use rustmatica::{Litematic, Region};
use std::collections::HashSet;
use std::io::{self, Write};

pub mod bamboo;
pub mod blocks;
pub mod cascade;
pub mod compact;
pub mod fishbone;
pub mod linear;
pub mod tapped;
pub mod torrent;

// Layout
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A queryable projection layout, generic over the context its parent declares.
///
/// `Ctx` is the shared knowledge a subtree is read within: a parent picks the
/// type and hands out one value, and every query receives a borrow of it, so a
/// leaf can read what only its ancestors know without holding its own copy. A
/// layout whose context is erased implements `Layout<()>`, the context of the
/// writer and of every finished layout.
///
/// Every query is named `*_with` and reads a borrow of `Ctx`. A layout whose
/// context is `()` gets the plain queries and the projection materialization of
/// [`LayoutExt`] for free, so a finished layout reads as `layout.size()`.
pub trait Layout<Ctx = ()> {
    /// Block at the given world position, read within `ctx`, assumed in bounds.
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState>;

    /// Total size of the bounding box, read within `ctx`.
    fn size_with(&self, ctx: &Ctx) -> BlockPos;

    /// Block at the given world position; `None` when it misses the bounding box.
    fn get_block_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        match self.contains_with(pos, ctx) {
            true => self.block_at_with(pos, ctx),
            false => None,
        }
    }

    /// Block at the given world position, or air where the layout is empty.
    fn get_block_or_air_with(&self, pos: BlockPos, ctx: &Ctx) -> BlockState {
        self.get_block_with(pos, ctx).unwrap_or_else(air)
    }

    /// Whether `pos` is inside the bounding box.
    fn contains_with(&self, pos: BlockPos, ctx: &Ctx) -> bool {
        let size = self.size_with(ctx);
        (0..size.x).contains(&pos.x) && (0..size.y).contains(&pos.y) && (0..size.z).contains(&pos.z)
    }

    /// The block states this instance owns: a set containing every state that
    /// [`block_at_with`](Self::block_at_with) can return, empty cells counted as
    /// air.
    ///
    /// A layout may over-approximate, so declaring it spares the writer a
    /// whole-box discovery walk; the default does exactly that walk via
    /// [`get_block_or_air_with`](Self::get_block_or_air_with). The writer alone
    /// fixes the id order, so the set need not be ordered.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let size = self.size_with(ctx);
        let at = |(y, z, x)| self.get_block_or_air_with(BlockPos::new(x, y, z), ctx);
        // Insert one by one: `collect` would reserve the box's whole volume
        // from the iterator's size hint, however few blocks it ends up holding.
        let mut palette = HashSet::new();
        for block in iproduct!(0..size.y, 0..size.z, 0..size.x).map(at) {
            palette.insert(block);
        }
        palette
    }
}

/// The plain surface of a finished layout: the queries of [`Layout`] with its
/// context fixed to `()`, plus materialization into a projection.
///
/// Every `Layout<()>` gets this for free, so a finished layout reads as
/// `layout.size()` or `layout.get_block(pos)` rather than spelling out the
/// empty context. A layout still carrying a context has only the `*_with`
/// queries.
pub trait LayoutExt: Layout<()> {
    /// Block at the given world position, assumed in bounds.
    fn block_at(&self, pos: BlockPos) -> Option<BlockState> {
        self.block_at_with(pos, &())
    }

    /// Total size of the bounding box.
    fn size(&self) -> BlockPos {
        self.size_with(&())
    }

    /// Block at the given world position; `None` when it misses the bounding box.
    fn get_block(&self, pos: BlockPos) -> Option<BlockState> {
        self.get_block_with(pos, &())
    }

    /// Block at the given world position, or air where the layout is empty.
    fn get_block_or_air(&self, pos: BlockPos) -> BlockState {
        self.get_block_or_air_with(pos, &())
    }

    /// Whether `pos` is inside the bounding box.
    fn contains(&self, pos: BlockPos) -> bool {
        self.contains_with(pos, &())
    }

    /// The block states this instance owns.
    fn block_palette(&self) -> HashSet<BlockState> {
        self.block_palette_with(&())
    }

    /// Writes this layout to `sink` as a gzip-compressed litematic projection,
    /// described by `metadata`.
    fn as_litematic<W: Write>(&self, metadata: Metadata, sink: W) -> io::Result<()>
    where
        Self: Sized,
    {
        schematic::write(self, metadata, sink)
    }

    /// Builds this layout as a `rustmatica` litematic, described by `metadata`.
    ///
    /// The old in-memory path: the whole projection is materialized before it is
    /// written, so a full song costs gigabytes. It stays only to check the
    /// streaming writer against it.
    #[deprecated(note = "materializes the whole projection; use `as_litematic`")]
    fn as_rustmatica(&self, metadata: Metadata) -> Litematic {
        let size = self.size();
        let mut region = Region::new(metadata.name.to_string(), BlockPos::ORIGIN, size);
        for (y, z, x) in iproduct!(0..size.y, 0..size.z, 0..size.x) {
            let pos = BlockPos::new(x, y, z);
            region.set_block(pos, self.get_block_or_air(pos).into());
        }
        region.as_litematic(
            metadata.description.to_string(),
            metadata.author.to_string(),
        )
    }
}

impl<T: Layout<()>> LayoutExt for T {}

impl<Ctx> Layout<Ctx> for () {
    fn block_at_with(&self, _pos: BlockPos, _ctx: &Ctx) -> Option<BlockState> {
        None
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        BlockPos::ORIGIN
    }
}

impl<Ctx> Layout<Ctx> for Box<dyn Layout<Ctx> + '_> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.as_ref().block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.as_ref().size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.as_ref().block_palette_with(ctx)
    }
}

/// Implements [`Layout`] on a wrapper by forwarding every query to one of its
/// fields: the first, `.0`, unless a field name or index is given as the third
/// argument.
///
/// Rust rejects the blanket impl that would make this automatic: it cannot rule
/// out a downstream impl overlapping a hand-written one, however private the
/// wrapper, so each wrapper states its delegation here in one line.
///
/// Exported for downstream crates, which use it the same way:
/// `forward_layout!(MyWrapper, ());` forwards to `.0` and
/// `forward_layout!(MyWrapper, (), inner);` to `inner`.
#[macro_export]
macro_rules! forward_layout {
    ($ty:ty, $ctx:ty) => {
        $crate::forward_layout!($ty, $ctx, 0);
    };
    ($ty:ty, $ctx:ty, $field:tt) => {
        impl $crate::layout::Layout<$ctx> for $ty {
            fn block_at_with(
                &self,
                pos: ::mcdata::util::BlockPos,
                ctx: &$ctx,
            ) -> Option<$crate::BlockState> {
                self.$field.block_at_with(pos, ctx)
            }

            fn size_with(&self, ctx: &$ctx) -> ::mcdata::util::BlockPos {
                self.$field.size_with(ctx)
            }

            fn block_palette_with(
                &self,
                ctx: &$ctx,
            ) -> ::std::collections::HashSet<$crate::BlockState> {
                self.$field.block_palette_with(ctx)
            }
        }
    };
}

/// Binds a context to a subtree, erasing it into [`Layout<()>`](Layout).
///
/// The context becomes an ordinary field rather than a borrow, so a finished
/// subtree carries no lifetime and every leaf below reaches the shared
/// knowledge through its queries alone.
pub struct Provided<Ctx, L> {
    ctx: Ctx,
    inner: L,
}

impl<Ctx, L> Provided<Ctx, L> {
    /// Binds `ctx` to `inner`, which was built within it.
    pub fn new(ctx: Ctx, inner: L) -> Self {
        Self { ctx, inner }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<()> for Provided<Ctx, L> {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        self.inner.block_at_with(pos, &self.ctx)
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        self.inner.size_with(&self.ctx)
    }

    fn block_palette_with(&self, _ctx: &()) -> HashSet<BlockState> {
        self.inner.block_palette_with(&self.ctx)
    }
}

// Arranged & EdgeArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that arranges sub-layouts along an [`Axis`].
///
/// Query cost is O(log n) with `n` the number of sub-layouts: [`Layout::block_at_with`]
/// locates the containing band via a binary search over the anchors, which are
/// sorted along the arrangement axis.
pub struct Arranged<L> {
    bands: Vec<(L, BlockPos)>,
    size: BlockPos,
}

impl<L> Arranged<L> {
    pub fn new<Ctx, I>(layouts: I, axis: Axis, gap: u32, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        let unit: Mask = axis.unit();
        let gap_vec: BlockPos = unit * gap as i32;
        let mut cursor: BlockPos = -gap_vec;
        let mut extent: BlockPos = BlockPos::ORIGIN;

        let placed = layouts.into_iter().map(|layout| {
            let size: BlockPos = layout.size_with(ctx);
            let anchor: BlockPos = cursor + gap_vec;
            cursor = anchor + unit * size;
            extent = include(extent, size);
            (layout, anchor)
        });

        let bands = placed.collect();
        let size = include(include(cursor, BlockPos::ORIGIN), extent);
        Self { bands, size }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Arranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let index = self
            .bands
            .partition_point(|(_, a)| a.y <= pos.y && a.z <= pos.z && a.x <= pos.x)
            .checked_sub(1)?;

        let (layout, anchor) = &self.bands[index];
        let local = BlockPos::new(pos.x - anchor.x, pos.y - anchor.y, pos.z - anchor.z);
        layout.get_block_with(local, ctx)
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of every band's palette.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = HashSet::new();
        for (layout, _) in &self.bands {
            palette.extend(layout.block_palette_with(ctx));
        }
        palette
    }
}

/// Like [`Arranged`], but aligns sub-layouts by their far-edge on cross-axes.
pub struct EdgeArranged<L>(Reverse<Arranged<Reverse<L>>>);

impl<L> EdgeArranged<L> {
    /// `align` is passed to both inner (per-sub-layout) and outer (whole) Reverse.
    pub fn new<Ctx, I>(layouts: I, axis: Axis, gap: u32, align: Mask, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        let reversed = layouts.into_iter().map(|l| Reverse::new(l, align, ctx));
        let arranged = Arranged::new(reversed, axis, gap, ctx);
        Self(Reverse::new(arranged, align, ctx))
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for EdgeArranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.0.block_at_with(pos, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.0.size_with(ctx)
    }

    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.0.block_palette_with(ctx)
    }
}

// EvenlyArranged
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Evenly arranges sub-layouts along the direction of `pitch`: item `i` is
/// anchored at `i * pitch`, a monotone linear lattice of equal spacing.
///
/// Query cost is O(k) with `k` the candidate window width, independent of
/// the item count. The window is the least that still reaches every item's
/// covering cell, so `k` is minimal. `pitch` must be non-zero; an empty
/// arrangement covers nothing and reports zero size.
pub struct EvenlyArranged<L> {
    /// Sub-layouts in lattice order; item `i` sits at `i * pitch`.
    items: Vec<L>,
    /// Non-zero step between item anchors.
    pitch: BlockPos,
    /// Shift so indices can go negative: `min(pitch, 0) * (len - 1)`.
    anchor: BlockPos,
    /// `pitch.dot(pitch)`, always positive.
    spacing: i32,
    /// Least `pitch.dot(local_pos - i * pitch)` over the integer points of one
    /// item box, so `top` never overshoots the highest covering item.
    bias: i32,
    /// Least candidate window width minus one that still reaches the highest
    /// covering item, `floor((L - M) / spacing)` with `L = |pitch|.extent` and
    /// `M = |pitch|.ones`.
    span: i32,
    /// Bounding box size: `pitch.abs() * (len - 1) + extent`, zero when empty.
    size: BlockPos,
}

impl<L> EvenlyArranged<L> {
    pub fn new<Ctx, I>(items: I, pitch: BlockPos, ctx: &Ctx) -> Self
    where
        I: IntoIterator<Item = L>,
        L: Layout<Ctx>,
    {
        assert!(pitch != BlockPos::ORIGIN, "pitch must be non-zero");
        let items = Vec::from_iter(items);
        let extent = items
            .iter()
            .map(|item| item.size_with(ctx))
            .fold(BlockPos::ORIGIN, include);
        let far = (items.len() as i32 - 1).max(0);

        let inner = include(extent - BlockPos::new(1, 1, 1), BlockPos::ORIGIN);
        let fore = include(pitch, BlockPos::ORIGIN);
        let back = include(-pitch, BlockPos::ORIGIN);
        let spacing = pitch.dot(pitch);

        let ahead = fore.dot(inner);
        let behind = back.dot(inner);
        Self {
            items,
            pitch,
            anchor: -(back * far),
            spacing,
            bias: -behind,
            span: (ahead + behind) / spacing,
            size: pitch.abs() * far + extent,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for EvenlyArranged<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let local_pos = pos + self.anchor;
        let offset = self.pitch.dot(local_pos);
        let top = (offset - self.bias)
            .div_euclid(self.spacing)
            .min(self.items.len() as i32 - 1);
        let bottom = (top - self.span).max(0);
        (bottom..=top)
            .rev()
            .find_map(|i| self.items[i as usize].get_block_with(local_pos - self.pitch * i, ctx))
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of every item's palette.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = HashSet::new();
        for item in &self.items {
            palette.extend(item.block_palette_with(ctx));
        }
        palette
    }
}

// Overlaid
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Overlaps two positioned sub-layouts, the first covering the second, where a
/// non-negative anchor is an offset from the origin and a negative one an
/// offset from `size + 1`.
pub struct Overlaid<A, B> {
    first: (BlockPos, A),
    second: (BlockPos, B),
    size: BlockPos,
}

impl<A, B> Overlaid<A, B> {
    pub fn new<Ctx>(
        first_anchor: BlockPos,
        first: A,
        second_anchor: BlockPos,
        second: B,
        size: BlockPos,
        ctx: &Ctx,
    ) -> Self
    where
        A: Layout<Ctx>,
        B: Layout<Ctx>,
    {
        fn anchor(anchor: BlockPos, inner: BlockPos, size: BlockPos) -> BlockPos {
            let axis = |a: i32, i: i32, s: i32| if a < 0 { s - i + a + 1 } else { a };
            let x = axis(anchor.x, inner.x, size.x);
            let y = axis(anchor.y, inner.y, size.y);
            let z = axis(anchor.z, inner.z, size.z);
            BlockPos::new(x, y, z)
        }
        Self {
            first: (anchor(first_anchor, first.size_with(ctx), size), first),
            second: (anchor(second_anchor, second.size_with(ctx), size), second),
            size,
        }
    }
}

impl<Ctx, A: Layout<Ctx>, B: Layout<Ctx>> Layout<Ctx> for Overlaid<A, B> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let (anchor, layout) = &self.first;
        layout.get_block_with(pos - *anchor, ctx).or_else(|| {
            let (anchor, layout) = &self.second;
            layout.get_block_with(pos - *anchor, ctx)
        })
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The union of both sub-layouts' palettes.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = self.first.1.block_palette_with(ctx);
        palette.extend(self.second.1.block_palette_with(ctx));
        palette
    }
}

// Reverse
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Mirror-reverse a layout along given axes. Block facing unchanged:
/// the layout has no block-internal facing transform, so the block's own
/// orientation state is never dictated by it.
pub struct Reverse<L> {
    /// The wrapped layout; queries are mirrored into its local space.
    layout: L,
    /// Per-axis mirror offset `sign * (size - 1)`.
    bias: BlockPos,
    /// Per-axis mirror factor: `-1` on reversed axes, `+1` elsewhere.
    factor: BlockPos,
}

impl<L> Reverse<L> {
    pub fn new<Ctx>(layout: L, sign: Mask, ctx: &Ctx) -> Self
    where
        L: Layout<Ctx>,
    {
        let bias = sign * (layout.size_with(ctx) - BlockPos::new(1, 1, 1));
        let factor = BlockPos::new(1, 1, 1) - sign * 2;
        Self {
            layout,
            bias,
            factor,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Reverse<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let orig = BlockPos::new(
            self.bias.x + self.factor.x * pos.x,
            self.bias.y + self.factor.y * pos.y,
            self.bias.z + self.factor.z * pos.z,
        );
        self.layout.block_at_with(orig, ctx)
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        self.layout.size_with(ctx)
    }

    /// The inner palette: mirroring moves cells, it draws no new block.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.layout.block_palette_with(ctx)
    }
}

// WithFloor
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that adds a floor layer beneath another layout.
pub struct WithFloor<L> {
    layout: L,
    full: bool,
}

impl<L> WithFloor<L> {
    /// Whether the floor fully covers the entire bounding box.
    /// When `false`, only positions with a gravity block above get a floor.
    pub fn new(layout: L, full: bool) -> Self {
        Self { layout, full }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for WithFloor<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        let inner = |pos: BlockPos| self.layout.block_at_with(pos, ctx);
        match pos.y {
            0 if self.full => Some(floor_block()),
            0 => inner(pos)
                .filter(|b| b.needs_floor())
                .map(|_| floor_block()),
            _ => inner(BlockPos::new(pos.x, pos.y - 1, pos.z)),
        }
    }

    fn size_with(&self, ctx: &Ctx) -> BlockPos {
        let size = self.layout.size_with(ctx);
        BlockPos::new(size.x, size.y + 1, size.z)
    }

    /// The inner palette plus the floor block, the only block this wrapper adds
    /// on top of what the inner layout draws.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        let mut palette = self.layout.block_palette_with(ctx);
        palette.insert(floor_block());
        palette
    }
}

/// Whether a block state is a gravity block that needs floor support.
pub trait NeedsFloor {
    /// Whether this block state needs a floor to hold it up.
    fn needs_floor(&self) -> bool;
}

impl NeedsFloor for BlockState {
    fn needs_floor(&self) -> bool {
        self.name == "minecraft:sand"
    }
}

// Clipped
//
// ++++++++++++============++++++++++++============++++++++++++============

/// A layout wrapper that clips the inner layout along each axis by `pos`.
///
/// A non-negative component `n` shifts the inner layout by `n` and shrinks
/// the size by `n`; a negative component keeps the inner layout in place and
/// shrinks the size by `-n`.
pub struct Clipped<L> {
    layout: L,
    anchor: BlockPos,
    size: BlockPos,
}

impl<L> Clipped<L> {
    pub fn new<Ctx>(layout: L, pos: BlockPos, ctx: &Ctx) -> Self
    where
        L: Layout<Ctx>,
    {
        let anchor = include(pos, BlockPos::ORIGIN);
        let size = layout.size_with(ctx) - pos.abs();

        Self {
            layout,
            anchor,
            size,
        }
    }
}

impl<Ctx, L: Layout<Ctx>> Layout<Ctx> for Clipped<L> {
    fn block_at_with(&self, pos: BlockPos, ctx: &Ctx) -> Option<BlockState> {
        self.layout.block_at_with(pos + self.anchor, ctx)
    }

    fn size_with(&self, _ctx: &Ctx) -> BlockPos {
        self.size
    }

    /// The inner palette: clipping only drops cells, it draws no new block.
    fn block_palette_with(&self, ctx: &Ctx) -> HashSet<BlockState> {
        self.layout.block_palette_with(ctx)
    }
}

// Axis
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Which spatial direction sub-layouts are placed along.
#[derive(Clone, Copy)]
pub enum Axis {
    /// East-west axis (X).
    Easting,
    /// Vertical axis (Y).
    Elevation,
    /// South-north axis (Z).
    Southing,
}

impl Axis {
    /// Unit mask vector for this axis.
    pub fn unit(self) -> Mask {
        match self {
            Axis::Easting => Mask::new(BlockPos::new(1, 0, 0)).unwrap(),
            Axis::Elevation => Mask::new(BlockPos::new(0, 1, 0)).unwrap(),
            Axis::Southing => Mask::new(BlockPos::new(0, 0, 1)).unwrap(),
        }
    }
}

// Mask
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Component-wise mask (0 or 1 on each axis) for BlockPos operations.
///
/// Unlike `BlockPos * BlockPos` (cross product), `Mask * BlockPos` is component-wise.
#[derive(Clone, Copy)]
pub struct Mask(BlockPos);

impl Mask {
    pub fn new(sign: BlockPos) -> Option<Self> {
        match (sign.x == 0 || sign.x == 1)
            && (sign.y == 0 || sign.y == 1)
            && (sign.z == 0 || sign.z == 1)
        {
            true => Some(Self(sign)),
            false => None,
        }
    }

    /// Unwrap into the underlying [`BlockPos`].
    pub fn into_inner(self) -> BlockPos {
        self.0
    }
}

impl std::ops::Mul<BlockPos> for Mask {
    type Output = BlockPos;

    fn mul(self, rhs: BlockPos) -> BlockPos {
        BlockPos::new(self.0.x * rhs.x, self.0.y * rhs.y, self.0.z * rhs.z)
    }
}

impl std::ops::Mul<i32> for Mask {
    type Output = BlockPos;

    fn mul(self, rhs: i32) -> BlockPos {
        self.0 * rhs
    }
}

// Dot
//
// ++++++++++++============++++++++++++============++++++++++++============

/// The scalar projection of `v` onto `self`, treating both as vectors.
trait Dot {
    fn dot(self, v: BlockPos) -> i32;
}

impl Dot for BlockPos {
    fn dot(self, v: BlockPos) -> i32 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }
}

impl Dot for Mask {
    fn dot(self, v: BlockPos) -> i32 {
        self.into_inner().dot(v)
    }
}

// Helpers
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Component-wise maximum of two [`BlockPos`].
fn include(a: BlockPos, b: BlockPos) -> BlockPos {
    BlockPos::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z))
}

fn air() -> BlockState {
    BlockState::air()
}

fn floor_block() -> BlockState {
    BlockState {
        name: "minecraft:gray_stained_glass".into(),
        properties: Default::default(),
    }
}

// Tests
//
// ++++++++++++============++++++++++++============++++++++++++============

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid cuboid filling its whole box.
    struct Solid(BlockPos);

    impl Layout<()> for Solid {
        fn block_at_with(&self, pos: BlockPos, ctx: &()) -> Option<BlockState> {
            self.contains_with(pos, ctx).then(air)
        }

        fn size_with(&self, _ctx: &()) -> BlockPos {
            self.0
        }
    }

    fn build(sizes: &[(i32, i32, i32)], pitch: (i32, i32, i32)) -> EvenlyArranged<Solid> {
        let items = sizes.iter().map(|&(x, y, z)| Solid(BlockPos::new(x, y, z)));
        EvenlyArranged::new(items, BlockPos::new(pitch.0, pitch.1, pitch.2), &())
    }

    /// Whether any item covers `pos`, by scanning the whole lattice.
    fn covered(arranged: &EvenlyArranged<Solid>, pos: BlockPos) -> bool {
        let local = pos + arranged.anchor;
        arranged
            .items
            .iter()
            .enumerate()
            .any(|(i, item)| item.get_block(local - arranged.pitch * i as i32).is_some())
    }

    /// Pitch and item sizes sweeping positive, negative, mixed and overlapping
    /// lattices.
    const CASES: [(&[(i32, i32, i32)], (i32, i32, i32)); 8] = [
        (&[(5, 1, 1); 4], (2, 0, 0)),
        (&[(5, 1, 1); 4], (-2, 0, 0)),
        (&[(5, 5, 1); 4], (1, 1, 0)),
        (&[(3, 3, 2); 4], (0, 0, 2)),
        (&[(3, 3, 2); 4], (0, 0, -2)),
        (&[(2, 6, 2); 4], (0, 4, 0)),
        (&[(5, 1, 1), (3, 1, 1), (7, 1, 1), (2, 1, 1)], (3, 0, 0)),
        (&[(4, 4, 3); 4], (2, 3, -1)),
    ];

    /// Every cell any item covers is found, so the window never misses.
    #[test]
    fn window_covers_every_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let size = arranged.size();
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let hit = arranged.get_block(pos).is_some();
                        assert_eq!(hit, covered(&arranged, pos), "{pos:?} at {pitch:?}");
                    }
                }
            }
        }
    }

    /// The window always reaches the highest covering item, so no scan can start
    /// below a cell it must find.
    #[test]
    fn window_reaches_highest_covering_item() {
        for &(sizes, pitch) in &CASES {
            let arranged = build(sizes, pitch);
            let size = arranged.size();
            let mut needed = 0;
            for z in 0..size.z {
                for y in 0..size.y {
                    for x in 0..size.x {
                        let pos = BlockPos::new(x, y, z);
                        let local = pos + arranged.anchor;
                        let top = arranged.pitch.dot(local) - arranged.bias;
                        let top = top
                            .div_euclid(arranged.spacing)
                            .min(arranged.items.len() as i32 - 1);
                        let highest = (0..arranged.items.len()).rev().find(|&i| {
                            arranged.items[i]
                                .get_block(local - arranged.pitch * i as i32)
                                .is_some()
                        });
                        if let Some(highest) = highest {
                            needed = needed.max(top - highest as i32);
                        }
                    }
                }
            }
            assert!(
                arranged.span >= needed,
                "{pitch:?}: {needed} > {}",
                arranged.span
            );
        }
    }
}
