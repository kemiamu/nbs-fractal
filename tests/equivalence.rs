//! The streaming writer must reproduce the rustmatica reference projection.

use itertools::iproduct;
use mcdata::{BlockState, util::BlockPos};
use nbs_fractal::GenericBlockState;
use nbs_fractal::layout::Layout;
use nbs_fractal::schematic::Metadata;
use rustmatica::{Litematic, Region};

const NAME: &str = "Note Block Track Schematic";

/// A block state from a name and a list of properties.
fn block(name: &'static str, props: &[(&'static str, &'static str)]) -> GenericBlockState {
    GenericBlockState {
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

impl Layout for Sample {
    fn block_at(&self, pos: BlockPos) -> Option<GenericBlockState> {
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

    fn size(&self) -> BlockPos {
        SIZE
    }
}

/// The reference bytes, built the way the old in-memory path built them.
fn reference(layout: &Sample) -> Vec<u8> {
    let size = Layout::size(layout);
    let mut region: Region<mcdata::GenericBlockState> = Region::new(NAME, BlockPos::ORIGIN, size);
    for (y, z, x) in iproduct!(0..size.y, 0..size.z, 0..size.x) {
        let pos = BlockPos::new(x, y, z);
        let state = layout.block_at(pos).map(Into::into);
        region.set_block(pos, state.unwrap_or_else(mcdata::GenericBlockState::air));
    }
    let litematic: Litematic = region.as_litematic("Equivalence", "nbs-fractal");
    litematic.to_bytes().unwrap()
}

/// The bytes the streaming writer produces.
fn written(layout: &Sample) -> Vec<u8> {
    let mut bytes = Vec::new();
    let metadata = Metadata {
        name: NAME,
        description: "Equivalence",
        author: "nbs-fractal",
    };
    layout.as_litematic(metadata, &mut bytes).unwrap();
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
