//! Block state projections for notes and instruments.

use crate::BlockState;
use rsnbs::note::{ImitateInstrument, Instrument, Tone};
use std::{borrow::Cow, collections::BTreeMap};

/// whether this tone is renderable: built-in instrument with a minecraft note.
fn is_valid(tone: &Tone) -> bool {
    !matches!(tone.instrument, Instrument::Custom(_)) && tone.key.minecraft_note().is_some()
}

// Tone block states
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Block states projected from a [`Tone`].
pub trait ToneBlocks {
    /// returns the minecraft note block block state for this tone.
    fn note_block_state(&self) -> Option<BlockState>;

    /// returns the block under the note block for this instrument's sound.
    fn instrument_block_state(&self) -> Option<BlockState>;

    /// returns the mob head block for this tone, if it is a mob head instrument.
    fn head_block_state(&self) -> Option<BlockState>;
}

impl ToneBlocks for Tone {
    fn note_block_state(&self) -> Option<BlockState> {
        let note = self.key.minecraft_note()?;
        let instr = self.instrument.note_property();
        let properties = From::from([
            ("note".into(), note.to_string().into()),
            ("powered".into(), "false".into()),
            ("instrument".into(), instr.into()),
        ]);
        Some(BlockState {
            name: "minecraft:note_block".into(),
            properties,
        })
    }

    fn instrument_block_state(&self) -> Option<BlockState> {
        if !is_valid(self) || matches!(self.instrument, Instrument::Imitate(_)) {
            return None;
        }
        let block = self.instrument.block_resource().unwrap();
        let properties = match self.instrument {
            Instrument::Banjo => From::from([("axis".into(), "y".into())]),
            _ => BTreeMap::new(),
        };
        Some(BlockState {
            name: block.into(),
            properties,
        })
    }

    fn head_block_state(&self) -> Option<BlockState> {
        if !is_valid(self) || !matches!(self.instrument, Instrument::Imitate(_)) {
            return None;
        }
        let block = self.instrument.block_resource().unwrap();
        Some(BlockState {
            name: block.into(),
            properties: BTreeMap::new(),
        })
    }
}

// Instrument block states
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Block states projected from an [`Instrument`].
pub trait InstrumentBlocks {
    /// returns the minecraft instrument property string for note block state.
    fn note_property(&self) -> &'static str;

    /// returns the block resource name for this instrument.
    fn block_resource(&self) -> Option<&'static str>;

    /// returns the block under the note block for this instrument's sound.
    fn instrument_block(&self) -> Option<BlockState>;

    /// returns the mob head block for this instrument, if it is a mob head instrument.
    fn head_block(&self) -> Option<BlockState>;
}

impl InstrumentBlocks for Instrument {
    fn note_property(&self) -> &'static str {
        match self {
            Self::Harp => "harp",
            Self::DoubleBass => "bass",
            Self::BassDrum => "basedrum",
            Self::SnareDrum => "snare",
            Self::Click => "hat",
            Self::Guitar => "guitar",
            Self::Flute => "flute",
            Self::Bell => "bell",
            Self::Chime => "chime",
            Self::Xylophone => "xylophone",
            Self::IronXylophone => "iron_xylophone",
            Self::CowBell => "cow_bell",
            Self::Didgeridoo => "didgeridoo",
            Self::Bit => "bit",
            Self::Banjo => "banjo",
            Self::Pling => "pling",
            Self::Trumpet => "trumpet",
            Self::TrumpetExposed => "trumpet_exposed",
            Self::TrumpetWeathered => "trumpet_weathered",
            Self::TrumpetOxidized => "trumpet_oxidized",
            Self::Imitate(instrument) => instrument.note_property(),
            Self::Custom(_) => "custom",
        }
    }

    fn block_resource(&self) -> Option<&'static str> {
        Some(match self {
            Self::Harp => "minecraft:dirt",
            Self::DoubleBass => "minecraft:oak_planks",
            Self::BassDrum => "minecraft:stone",
            Self::SnareDrum => "minecraft:sand",
            Self::Click => "minecraft:glass",
            Self::Guitar => "minecraft:white_wool",
            Self::Flute => "minecraft:clay",
            Self::Bell => "minecraft:gold_block",
            Self::Chime => "minecraft:packed_ice",
            Self::Xylophone => "minecraft:bone_block",
            Self::IronXylophone => "minecraft:iron_block",
            Self::CowBell => "minecraft:soul_sand",
            Self::Didgeridoo => "minecraft:pumpkin",
            Self::Bit => "minecraft:emerald_block",
            Self::Banjo => "minecraft:hay_block",
            Self::Pling => "minecraft:glowstone",
            Self::Trumpet => "minecraft:waxed_copper_block",
            Self::TrumpetExposed => "minecraft:waxed_exposed_copper",
            Self::TrumpetWeathered => "minecraft:waxed_weathered_copper",
            Self::TrumpetOxidized => "minecraft:waxed_oxidized_copper",
            Self::Imitate(instrument) => instrument.block_resource(),
            Self::Custom(_) => return None,
        })
    }

    fn instrument_block(&self) -> Option<BlockState> {
        if matches!(self, Self::Imitate(_)) {
            return None;
        }
        let block = self.block_resource()?;
        Some(BlockState {
            name: Cow::Borrowed(block),
            properties: BTreeMap::new(),
        })
    }

    fn head_block(&self) -> Option<BlockState> {
        if !matches!(self, Self::Imitate(_)) {
            return None;
        }
        let block = self.block_resource()?;
        Some(BlockState {
            name: block.into(),
            properties: BTreeMap::new(),
        })
    }
}

// ImitateInstrument block states
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Block states projected from an [`ImitateInstrument`].
pub trait ImitateBlocks {
    /// returns the minecraft instrument property string for note block state.
    fn note_property(self) -> &'static str;

    /// returns the block resource name for this mob head.
    fn block_resource(self) -> &'static str;
}

impl ImitateBlocks for ImitateInstrument {
    fn note_property(self) -> &'static str {
        match self {
            Self::Creeper => "creeper",
            Self::Skeleton => "skeleton",
            Self::Dragon => "ender_dragon",
            Self::WitherSkeleton => "wither_skeleton",
            Self::Piglin => "piglin",
            Self::Zombie => "zombie",
            Self::CustomHead => "custom_head",
        }
    }

    fn block_resource(self) -> &'static str {
        match self {
            Self::Creeper => "minecraft:creeper_head",
            Self::Skeleton => "minecraft:skeleton_skull",
            Self::Dragon => "minecraft:dragon_head",
            Self::WitherSkeleton => "minecraft:wither_skeleton_skull",
            Self::Piglin => "minecraft:piglin_head",
            Self::Zombie => "minecraft:zombie_head",
            Self::CustomHead => "minecraft:player_head",
        }
    }
}

// Block state helpers
//
// ++++++++++++============++++++++++++============++++++++++++============

/// Note block, or fallback on None.
pub fn note_block<T: AsRef<Tone>>(note: Option<T>, fallback: fn() -> BlockState) -> BlockState {
    note.and_then(|t| t.as_ref().note_block_state())
        .unwrap_or_else(fallback)
}

/// Instrument block, or fallback on None.
pub fn inst_block<T: AsRef<Tone>>(note: Option<T>, fallback: fn() -> BlockState) -> BlockState {
    note.and_then(|t| t.as_ref().instrument_block_state())
        .unwrap_or_else(fallback)
}

/// Base block that the circuitry is carried on.
pub fn chain_block() -> BlockState {
    BlockState {
        name: "minecraft:smooth_stone".into(),
        properties: Default::default(),
    }
}

/// Connection state of a redstone wire on one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireConn {
    /// No connection on this side.
    None,
    /// Connected to a wire beside it.
    Side,
    /// Connected to a wire above it.
    Up,
}

impl WireConn {
    /// Minecraft property value of this connection state.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Side => "side",
            Self::Up => "up",
        }
    }
}

/// Redstone wire connected on every side, unpowered.
pub fn redstone_wire() -> BlockState {
    wire_state(
        WireConn::Side,
        WireConn::Side,
        WireConn::Side,
        WireConn::Side,
        "0",
    )
}

/// Redstone wire with explicit connection states and signal strength.
///
/// These are the post-update states a placed wire settles into.
pub fn wire_state(
    west: WireConn,
    east: WireConn,
    north: WireConn,
    south: WireConn,
    power: impl Into<Cow<'static, str>>,
) -> BlockState {
    BlockState {
        name: "minecraft:redstone_wire".into(),
        properties: From::from([
            ("power".into(), power.into()),
            ("north".into(), north.name().into()),
            ("south".into(), south.name().into()),
            ("east".into(), east.name().into()),
            ("west".into(), west.name().into()),
        ]),
    }
}

/// In-game facing of a directional block.
///
/// Note: repeaters store the reverse of this value as their property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    /// Facing east.
    East,
    /// Facing west.
    West,
    /// Facing north.
    North,
    /// Facing south.
    South,
}

impl Facing {
    /// The opposite facing.
    pub fn invert(self) -> Self {
        match self {
            Self::East => Self::West,
            Self::West => Self::East,
            Self::North => Self::South,
            Self::South => Self::North,
        }
    }

    /// Minecraft property value of this facing.
    pub fn name(self) -> &'static str {
        match self {
            Self::East => "east",
            Self::West => "west",
            Self::North => "north",
            Self::South => "south",
        }
    }
}

/// Repeater block with delay, pointing direction, and powered state.
pub fn repeater(
    delay: impl Into<Cow<'static, str>>,
    toward: Facing,
    powered: bool,
    locked: bool,
) -> BlockState {
    let powered = if powered { "true" } else { "false" };
    let locked = if locked { "true" } else { "false" };
    BlockState {
        name: "minecraft:repeater".into(),
        properties: From::from([
            ("delay".into(), delay.into()),
            // NOTE: a repeater's facing is backwards from the way it points.
            //       Mojang's mysterious code?
            ("facing".into(), toward.invert().name().into()),
            ("locked".into(), locked.into()),
            ("powered".into(), powered.into()),
        ]),
    }
}

/// Observer block with facing.
pub fn observer(facing: impl Into<Cow<'static, str>>) -> BlockState {
    BlockState {
        name: "minecraft:observer".into(),
        properties: From::from([
            ("facing".into(), facing.into()),
            ("powered".into(), "false".into()),
        ]),
    }
}

/// Sticky piston block, not extended.
pub fn sticky_piston<T: Into<Cow<'static, str>>>(facing: T) -> BlockState {
    BlockState {
        name: "minecraft:sticky_piston".into(),
        properties: From::from([
            ("facing".into(), facing.into()),
            ("extended".into(), "false".into()),
        ]),
    }
}

/// Redstone block.
pub fn redstone_block() -> BlockState {
    BlockState {
        name: "minecraft:redstone_block".into(),
        properties: Default::default(),
    }
}

/// Redstone torch with lit state and optional facing.
pub fn redstone_torch<T: Into<Cow<'static, str>>>(facing: Option<T>, lit: bool) -> BlockState {
    let lit = if lit { "true" } else { "false" };
    let name = match facing.is_some() {
        true => "minecraft:redstone_wall_torch".into(),
        false => "minecraft:redstone_torch".into(),
    };
    let properties = match facing {
        Some(f) => From::from([("lit".into(), lit.into()), ("facing".into(), f.into())]),
        None => From::from([("lit".into(), lit.into())]),
    };
    BlockState { name, properties }
}
