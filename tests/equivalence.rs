//! The streaming writer must reproduce the rustmatica reference projection.

use itertools::iproduct;
use mcdata::util::BlockPos;
use nbs_fractal::BlockState;
use nbs_fractal::layout::{Layout, LayoutExt};
use nbs_fractal::schematic::Metadata;
use rustmatica::Litematic;

const NAME: &str = "Note Block Track Schematic";

/// A block state from a name and a list of properties.
fn block(name: &'static str, props: &[(&'static str, &'static str)]) -> BlockState {
    BlockState {
        name: name.into(),
        properties: props
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    }
}

/// A tiny layout exercising note blocks, instruments, and circuit blocks.
struct Sample;

const SIZE: BlockPos = BlockPos::new(4, 3, 3);

impl Layout<()> for Sample {
    fn block_at_with(&self, pos: BlockPos, _ctx: &()) -> Option<BlockState> {
        let harp = [("instrument", "harp"), ("note", "12"), ("powered", "false")];
        let repeater = [
            ("delay", "1"),
            ("facing", "north"),
            ("locked", "false"),
            ("powered", "false"),
        ];
        match (pos.x, pos.y, pos.z) {
            (1, 1, 1) => Some(block("minecraft:note_block", &harp)),
            (1, 0, 1) => Some(block("minecraft:dirt", &[])),
            (2, 0, 2) => Some(block("minecraft:repeater", &repeater)),
            (0, 2, 0) => Some(block("minecraft:barrier", &[("waterlogged", "false")])),
            _ => None,
        }
    }

    fn size_with(&self, _ctx: &()) -> BlockPos {
        SIZE
    }
}

/// The metadata both paths are given.
fn metadata() -> Metadata<'static> {
    Metadata {
        name: NAME,
        description: "Equivalence",
        author: "nbs-fractal",
    }
}

/// The bytes the old in-memory path produces.
#[allow(deprecated)]
fn reference(layout: &Sample) -> Vec<u8> {
    layout.as_rustmatica(metadata()).to_bytes().unwrap()
}

/// The bytes the streaming writer produces.
fn written(layout: &Sample) -> Vec<u8> {
    let mut bytes = Vec::new();
    layout.as_litematic(metadata(), &mut bytes).unwrap();
    bytes
}

/// Every cell's block, read back through rustmatica, in writing order.
///
/// The palette order is the writer's to choose, so the two projections are held
/// equal on the blocks they paint rather than on their bytes.
fn cells(bytes: &[u8]) -> Vec<mcdata::GenericBlockState> {
    let litematic: Litematic<mcdata::GenericBlockState> = Litematic::from_bytes(bytes).unwrap();
    let region = &litematic.regions[0];
    let size = region.size;
    iproduct!(0..size.y, 0..size.z, 0..size.x)
        .map(|(y, z, x)| region.get_block(BlockPos::new(x, y, z)).clone())
        .collect()
}

#[test]
fn matches_reference() {
    let layout = Sample;
    assert_eq!(cells(&written(&layout)), cells(&reference(&layout)));
}
