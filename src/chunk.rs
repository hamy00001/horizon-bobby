//! Fake chunk (`CachedChunk`) and section (`CachedSection`) representations.
//!
//! Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunk` and
//! `de.johni0702.minecraft.bobby.ext.FakeChunkData`, bridged with Horizon's
//! `horizon_world` and `horizon_protocol` chunk primitives.

use horizon_core::position::ChunkPos;
use horizon_protocol::packets::play::LevelChunkWithLight;
use horizon_world::chunk::{BlockProperties, BlockState, PalettedContainer};
use horizon_world::nbt::{Compound, JavaString, Tag};
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

/// Light nibble array size for a 16x16x16 section (4,096 blocks, 4 bits per block = 2,048 bytes).
pub const LIGHT_LAYER_BYTES: usize = 2048;

/// Amount by which sky light is reduced in tainted fake chunks (`FakeChunk.getFakeLightEngine`: `value - 5`).
pub const TAINT_LIGHT_OFFSET: i8 = -5;

/// A single 16x16x16 vertical section of a cached chunk column.
#[derive(Clone, Debug)]
pub struct CachedSection {
    /// Section Y coordinate (e.g., `-4..20` in the standard Overworld).
    pub y: i8,
    /// Block states paletted container (`4096` entries, `min_bits = 4`).
    pub block_states: PalettedContainer,
    /// Biomes paletted container (`64` entries, `min_bits = 1`).
    pub biomes: PalettedContainer,
    /// Packed 2,048-byte nibble array for block light (`None` if all dark / unlit).
    pub block_light: Option<Arc<[u8; LIGHT_LAYER_BYTES]>>,
    /// Packed 2,048-byte nibble array for sky light (`None` if omitted/derived).
    pub sky_light: Option<Arc<[u8; LIGHT_LAYER_BYTES]>>,
}

impl CachedSection {
    /// Creates an empty (all-air, plains biome) section at section `y`.
    pub fn empty(y: i8) -> Self {
        let air = BlockState {
            name: "minecraft:air".to_owned(),
            properties: BlockProperties::new(),
        }
        .to_nbt();
        let plains = Tag::String(JavaString::from("minecraft:plains"));
        Self {
            y,
            block_states: PalettedContainer::uniform(air, 4096, 4).expect("valid air container"),
            biomes: PalettedContainer::uniform(plains, 64, 1).expect("valid biome container"),
            block_light: None,
            sky_light: None,
        }
    }

    /// Returns whether this section contains only `minecraft:air` (or `minecraft:cave_air` / `minecraft:void_air`).
    pub fn is_only_air(&self) -> bool {
        if self.block_states.palette_len() == 1 {
            if let Ok(tag) = self.block_states.get(0) {
                return is_air_tag(tag);
            }
        }
        false
    }

    /// Reads the block state at section-local `(x, y, z)` in `0..16`.
    pub fn get_block_state(&self, x: usize, y: usize, z: usize) -> Option<BlockState> {
        let idx = (y & 15) << 8 | (z & 15) << 4 | (x & 15);
        let tag = self.block_states.get(idx).ok()?;
        BlockState::from_nbt(tag).ok()
    }

    /// Reads a 4-bit nibble (`0..=15`) from a 2,048-byte light layer at `(x, y, z)` in `0..16`.
    #[inline]
    pub fn get_nibble(layer: &[u8; LIGHT_LAYER_BYTES], x: usize, y: usize, z: usize) -> u8 {
        let idx = (y & 15) << 8 | (z & 15) << 4 | (x & 15);
        let byte = layer[idx >> 1];
        if idx & 1 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        }
    }

    /// Writes a 4-bit nibble (`0..=15`) into a 2,048-byte light layer at `(x, y, z)` in `0..16`.
    #[inline]
    pub fn set_nibble(
        layer: &mut [u8; LIGHT_LAYER_BYTES],
        x: usize,
        y: usize,
        z: usize,
        val: u8,
    ) {
        let idx = (y & 15) << 8 | (z & 15) << 4 | (x & 15);
        let nib = val & 0x0F;
        if idx & 1 == 0 {
            layer[idx >> 1] = (layer[idx >> 1] & 0xF0) | nib;
        } else {
            layer[idx >> 1] = (layer[idx >> 1] & 0x0F) | (nib << 4);
        }
    }
}

/// Checks if an NBT palette entry represents an air block state.
pub fn is_air_tag(tag: &Tag) -> bool {
    let name = match tag {
        Tag::String(s) => s.to_string_lossy(),
        Tag::Compound(_) => match tag.get("Name").or_else(|| tag.get("id")) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => return false,
        },
        _ => return false,
    };
    matches!(
        name.as_ref(),
        "minecraft:air" | "air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

/// A cached chunk column (`FakeChunk` in the original Bobby mod).
///
/// Sections are wrapped in [`Arc`] so taking a shallow snapshot of a real chunk
/// when the server unloads it (`ChunkSerializer.shallowCopy`) is near-zero-cost
/// on the client game thread.
#[derive(Clone, Debug)]
pub struct CachedChunk {
    /// Chunk coordinates `(x, z)`.
    pub pos: ChunkPos,
    /// Minimum section Y of the dimension (e.g. `-4` for Overworld `min_y = -64`).
    pub min_section_y: i32,
    /// Whether the dimension has skylight.
    pub has_skylight: bool,
    /// Whether lighting data is present and ready (`isLightOn`).
    pub is_light_on: bool,
    /// Timestamp (`age` tag in Bobby NBT) recording when this chunk was seen from the server.
    pub age: i64,
    /// Whether this chunk is currently rendered with Bobby's fake-chunk light taint.
    pub tainted: bool,
    /// Whether this fake chunk is scheduled to be replaced by an incoming real chunk.
    pub will_be_replaced: bool,
    /// Vertical sections ordered from `min_section_y` upward, wrapped in `Arc` for cheap shallow copies.
    pub sections: Vec<Arc<CachedSection>>,
    /// Heightmaps keyed by name (`WORLD_SURFACE`, `MOTION_BLOCKING`, `MOTION_BLOCKING_NO_LEAVES`, `OCEAN_FLOOR`).
    pub heightmaps: BTreeMap<String, Vec<i64>>,
    /// Saved block entity NBT compounds (`block_entities`).
    pub block_entities: Vec<Tag>,
    /// Retained original sky light layers before `set_tainted(true)` modified them, so toggling
    /// taint at runtime (`FakeChunk.setTainted`) restores exact original light values without reloading.
    pub(crate) untainted_sky_light: Option<Vec<Option<Arc<[u8; LIGHT_LAYER_BYTES]>>>>,
}

impl CachedChunk {
    /// Creates an empty `CachedChunk` with `section_count` sections starting at `min_section_y`.
    pub fn new(
        pos: ChunkPos,
        min_section_y: i32,
        section_count: usize,
        has_skylight: bool,
        age: i64,
    ) -> Self {
        let sections = (0..section_count)
            .map(|i| Arc::new(CachedSection::empty((min_section_y + i as i32) as i8)))
            .collect();
        Self {
            pos,
            min_section_y,
            has_skylight,
            is_light_on: true,
            age,
            tainted: false,
            will_be_replaced: false,
            sections,
            heightmaps: BTreeMap::new(),
            block_entities: Vec::new(),
            untainted_sky_light: None,
        }
    }

    /// Shallow copy of a real chunk for background saving (`ChunkSerializer.shallowCopy`),
    /// sharing `Arc<CachedSection>` buffers without cloning block/light arrays.
    pub fn shallow_copy(&self) -> Self {
        Self {
            pos: self.pos,
            min_section_y: self.min_section_y,
            has_skylight: self.has_skylight,
            is_light_on: self.is_light_on,
            age: self.age,
            tainted: false,
            will_be_replaced: false,
            sections: self.sections.clone(),
            heightmaps: self.heightmaps.clone(),
            block_entities: self.block_entities.clone(),
            untainted_sky_light: None,
        }
    }

    /// Strips block entities when `BobbyConfig::no_block_entities` is enabled.
    pub fn strip_block_entities(&mut self) {
        self.block_entities.clear();
    }

    /// Applies or removes the visual fake-chunk light taint (`FakeChunk.setTainted`).
    ///
    /// When `tainted` is `true`, sky light nibbles are reduced by `5` (clamped to `0..=15`)
    /// so fake chunks beyond the server's view distance can be visually distinguished.
    pub fn set_tainted(&mut self, value: bool) {
        if self.tainted == value {
            return;
        }
        self.tainted = value;
        if value {
            let originals: Vec<Option<Arc<[u8; LIGHT_LAYER_BYTES]>>> =
                self.sections.iter().map(|s| s.sky_light.clone()).collect();
            self.untainted_sky_light = Some(originals);

            for section_arc in &mut self.sections {
                if let Some(sky) = &section_arc.sky_light {
                    let mut darkened = [0u8; LIGHT_LAYER_BYTES];
                    for (dst, &src) in darkened.iter_mut().zip(sky.iter()) {
                        let lo = ((src & 0x0F) as i8 + TAINT_LIGHT_OFFSET).clamp(0, 15) as u8;
                        let hi = ((src >> 4) as i8 + TAINT_LIGHT_OFFSET).clamp(0, 15) as u8;
                        *dst = lo | (hi << 4);
                    }
                    let sec = Arc::make_mut(section_arc);
                    sec.sky_light = Some(Arc::new(darkened));
                }
            }
        } else if let Some(originals) = self.untainted_sky_light.take() {
            for (section_arc, orig_sky) in self.sections.iter_mut().zip(originals) {
                let sec = Arc::make_mut(section_arc);
                sec.sky_light = orig_sky;
            }
        }
    }

    /// Reads the effective sky light level (`0..=15`) at chunk-local `(x, y_block, z)`.
    pub fn get_sky_light(&self, x: usize, y_block: i32, z: usize) -> u8 {
        let sec_y = y_block.div_euclid(16);
        let idx = (sec_y - self.min_section_y) as isize;
        if idx < 0 || idx as usize >= self.sections.len() {
            return if self.has_skylight { 15 } else { 0 };
        }
        let local_y = y_block.rem_euclid(16) as usize;
        match &self.sections[idx as usize].sky_light {
            Some(layer) => CachedSection::get_nibble(layer, x, local_y, z),
            None => 0,
        }
    }

    /// Reads the effective block light level (`0..=15`) at chunk-local `(x, y_block, z)`.
    pub fn get_block_light(&self, x: usize, y_block: i32, z: usize) -> u8 {
        let sec_y = y_block.div_euclid(16);
        let idx = (sec_y - self.min_section_y) as isize;
        if idx < 0 || idx as usize >= self.sections.len() {
            return 0;
        }
        let local_y = y_block.rem_euclid(16) as usize;
        match &self.sections[idx as usize].block_light {
            Some(layer) => CachedSection::get_nibble(layer, x, local_y, z),
            None => 0,
        }
    }

    /// Constructs a `CachedChunk` directly from a Horizon multiplayer `LevelChunkWithLight` packet.
    ///
    /// `state_resolver` maps a wire global block-state ID (`u32`) to a `BlockState`,
    /// and `biome_resolver` maps a wire biome ID (`u32`) to a biome identifier string.
    pub fn from_network_packet<FS, FB>(
        packet: &LevelChunkWithLight,
        min_section_y: i32,
        section_count: usize,
        has_skylight: bool,
        biome_bits: u8,
        age: i64,
        mut state_resolver: FS,
        mut biome_resolver: FB,
    ) -> io::Result<Self>
    where
        FS: FnMut(u32) -> BlockState,
        FB: FnMut(u32) -> String,
    {
        let wire_sections = packet
            .chunk
            .sections(section_count, 16, biome_bits)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let light_layers = packet.light.layers(section_count + 2);
        let mut sections = Vec::with_capacity(section_count);

        for (i, wire_sec) in wire_sections.iter().enumerate() {
            let y = (min_section_y + i as i32) as i8;

            // Convert wire block states into NBT PalettedContainer.
            let air = BlockState {
                name: "minecraft:air".to_owned(),
                properties: BlockProperties::new(),
            }
            .to_nbt();
            let mut block_states = PalettedContainer::uniform(air, 4096, 4)?;
            for idx in 0..4096 {
                let state_id = wire_sec.states.get(idx) as u32;
                let tag = state_resolver(state_id).to_nbt();
                block_states.set(idx, tag)?;
            }

            // Convert wire biomes into NBT PalettedContainer.
            let plains = Tag::String(JavaString::from("minecraft:plains"));
            let mut biomes = PalettedContainer::uniform(plains, 64, 1)?;
            for idx in 0..64 {
                let biome_id = wire_sec.biomes.get(idx) as u32;
                let name = biome_resolver(biome_id);
                biomes.set(idx, Tag::String(JavaString::from(name.as_str())))?;
            }

            // Light section index `i + 1` corresponds to world section `i`
            // (since index 0 is the section below min_section_y).
            let (sky_light, block_light) = match light_layers.get(i + 1) {
                Some((sky, block)) => {
                    let sky_arr = sky.as_ref().and_then(|v| {
                        <[u8; LIGHT_LAYER_BYTES]>::try_from(v.as_slice())
                            .ok()
                            .map(Arc::new)
                    });
                    let block_arr = block.as_ref().and_then(|v| {
                        <[u8; LIGHT_LAYER_BYTES]>::try_from(v.as_slice())
                            .ok()
                            .map(Arc::new)
                    });
                    (sky_arr, block_arr)
                }
                None => (None, None),
            };

            sections.push(Arc::new(CachedSection {
                y,
                block_states,
                biomes,
                block_light,
                sky_light,
            }));
        }

        let mut heightmaps = BTreeMap::new();
        for (hm_type, words) in &packet.chunk.heightmaps {
            let name = match *hm_type {
                1 => "WORLD_SURFACE",
                2 => "WORLD_SURFACE_WG",
                3 => "OCEAN_FLOOR",
                4 => "MOTION_BLOCKING",
                5 => "MOTION_BLOCKING_NO_LEAVES",
                _ => continue,
            };
            heightmaps.insert(name.to_owned(), words.clone());
        }

        let mut block_entities = Vec::with_capacity(packet.chunk.block_entities.len());
        for info in &packet.chunk.block_entities {
            let bx = packet.x * 16 + i32::from(info.packed_xz >> 4);
            let by = i32::from(info.y);
            let bz = packet.z * 16 + i32::from(info.packed_xz & 15);
            let mut map = match &info.tag {
                Tag::Compound(c) => c.clone(),
                _ => Compound::new(),
            };
            map.insert(JavaString::from("x"), Tag::Int(bx));
            map.insert(JavaString::from("y"), Tag::Int(by));
            map.insert(JavaString::from("z"), Tag::Int(bz));
            block_entities.push(Tag::Compound(map));
        }

        Ok(Self {
            pos: ChunkPos::new(packet.x, packet.z),
            min_section_y,
            has_skylight,
            is_light_on: true,
            age,
            tainted: false,
            will_be_replaced: false,
            sections,
            heightmaps,
            block_entities,
            untainted_sky_light: None,
        })
    }
}
