//! The streaming writer must reproduce the rustmatica reference projection.

use itertools::iproduct;
use mcdata::{BlockState, GenericBlockState, util::BlockPos};
use nbs_fractal::layout::Layout;
use nbs_fractal::schematic::Metadata;
use rustmatica::{Litematic, Region};
use std::io::Read;

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
    let mut region: Region<GenericBlockState> = Region::new(NAME, BlockPos::ORIGIN, size);
    for (y, z, x) in iproduct!(0..size.y, 0..size.z, 0..size.x) {
        let pos = BlockPos::new(x, y, z);
        region.set_block(
            pos,
            layout.block_at(pos).unwrap_or_else(GenericBlockState::air),
        );
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

/// Decompresses and parses a projection, ignoring the creation timestamps.
fn parsed(bytes: &[u8]) -> fastnbt::Value {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    let mut value: fastnbt::Value = fastnbt::from_bytes(&raw).unwrap();
    if let fastnbt::Value::Compound(root) = &mut value {
        if let Some(fastnbt::Value::Compound(meta)) = root.get_mut("Metadata") {
            meta.insert("TimeCreated".into(), fastnbt::Value::Long(0));
            meta.insert("TimeModified".into(), fastnbt::Value::Long(0));
        }
    }
    value
}

#[test]
fn matches_reference() {
    let layout = Sample;
    assert_eq!(parsed(&written(&layout)), parsed(&reference(&layout)));
}
