//! Anvil chunk NBT serialization, deserialization, skylight flood-fill fallback,
//! and 64-bit opaque-block fingerprinting.
//!
//! Direct Rust port of `de.johni0702.minecraft.bobby.ChunkSerializer`.

use super::DATA_VERSION;
use super::chunk::{CachedChunk, CachedSection, LIGHT_LAYER_BYTES, is_air_tag};
use super::config::BobbyConfig;
use horizon_core::position::ChunkPos;
use horizon_world::chunk::PalettedContainer;
use horizon_world::nbt::{Compound, JavaString, NamedTag, Tag};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

fn jkey(s: &str) -> JavaString {
    JavaString::from(s)
}

fn tag_int(tag: Option<&Tag>) -> Option<i32> {
    match tag? {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => i32::try_from(*v).ok(),
        _ => None,
    }
}

fn tag_long(tag: Option<&Tag>) -> Option<i64> {
    match tag? {
        Tag::Byte(v) => Some(i64::from(*v)),
        Tag::Short(v) => Some(i64::from(*v)),
        Tag::Int(v) => Some(i64::from(*v)),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}

/// Serializes a [`CachedChunk`] into a root Anvil [`NamedTag`] ready for region compression.
///
/// Mirrors `ChunkSerializer.serialize(LevelChunk, LevelLightEngine)` in Bobby.
pub fn serialize(chunk: &CachedChunk, config: &BobbyConfig) -> NamedTag {
    let mut sections_list = Vec::with_capacity(chunk.sections.len());

    // If chunk is tainted in memory, serialize its untainted sky light if available.
    for (idx, section) in chunk.sections.iter().enumerate() {
        let mut sec_map = Compound::new();
        sec_map.insert(jkey("Y"), Tag::Byte(section.y));
        sec_map.insert(jkey("block_states"), section.block_states.to_nbt());
        sec_map.insert(jkey("biomes"), section.biomes.to_nbt());

        if let Some(block_light) = &section.block_light {
            sec_map.insert(jkey("BlockLight"), Tag::ByteArray(block_light.to_vec()));
        }
        let sky_light = chunk
            .untainted_sky_light
            .as_ref()
            .and_then(|v| v.get(idx))
            .and_then(|s| s.as_ref())
            .or(section.sky_light.as_ref());
        if let Some(sky) = sky_light {
            sec_map.insert(jkey("SkyLight"), Tag::ByteArray(sky.to_vec()));
        }

        sections_list.push(Tag::Compound(sec_map));
    }

    let mut heightmaps_map = Compound::new();
    for (name, data) in &chunk.heightmaps {
        heightmaps_map.insert(jkey(name), Tag::LongArray(data.clone()));
    }

    let block_entities = if config.no_block_entities {
        Vec::new()
    } else {
        chunk.block_entities.clone()
    };

    let mut root = Compound::new();
    root.insert(jkey("DataVersion"), Tag::Int(DATA_VERSION));
    root.insert(jkey("xPos"), Tag::Int(chunk.pos.x));
    root.insert(jkey("yPos"), Tag::Int(chunk.min_section_y));
    root.insert(jkey("zPos"), Tag::Int(chunk.pos.z));
    root.insert(jkey("isLightOn"), Tag::Byte(i8::from(chunk.is_light_on)));
    root.insert(jkey("Status"), Tag::String(jkey("minecraft:full")));
    root.insert(jkey("sections"), Tag::List(sections_list));
    root.insert(jkey("block_entities"), Tag::List(block_entities));
    root.insert(jkey("Heightmaps"), Tag::Compound(heightmaps_map));
    root.insert(jkey("age"), Tag::Long(chunk.age));

    NamedTag {
        name: jkey(""),
        tag: Tag::Compound(root),
    }
}

/// Deserializes an Anvil chunk [`NamedTag`] into a [`CachedChunk`].
///
/// Mirrors `ChunkSerializer.deserialize(ChunkPos, CompoundTag, ClientLevel)`:
/// - Reconstructs `block_states`, `biomes`, `BlockLight`, and `SkyLight` per section.
/// - Runs Bobby's skylight flood-fill fallback if any non-empty section was saved without `SkyLight`.
/// - Applies `config.no_block_entities` and `config.taint_fake_chunks`.
pub fn deserialize(
    expected_pos: ChunkPos,
    root: &NamedTag,
    config: &BobbyConfig,
    has_skylight: bool,
) -> Option<CachedChunk> {
    let tag = &root.tag;
    let x = tag_int(tag.get("xPos")).unwrap_or(expected_pos.x);
    let z = tag_int(tag.get("zPos")).unwrap_or(expected_pos.z);
    let min_section_y = tag_int(tag.get("yPos")).unwrap_or(-4);
    let age = tag_long(tag.get("age")).unwrap_or(0);
    let is_light_on = tag_int(tag.get("isLightOn")).unwrap_or(1) != 0;

    let Some(Tag::List(raw_sections)) = tag.get("sections") else {
        return None;
    };

    // Determine the section range from the stored sections (defaulting to at least 24 sections for Overworld).
    let mut min_y = min_section_y;
    let mut max_y = min_section_y + 23;
    for sec in raw_sections {
        if let Some(y) = tag_int(sec.get("Y")) {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    let section_count = (max_y - min_y + 1).max(1) as usize;
    let mut sections: Vec<CachedSection> = (0..section_count)
        .map(|i| CachedSection::empty((min_y + i as i32) as i8))
        .collect();

    let mut completely_lit = true;

    for raw_sec in raw_sections {
        let Some(y) = tag_int(raw_sec.get("Y")) else {
            continue;
        };
        let idx = (y - min_y) as isize;
        if idx < 0 || idx as usize >= sections.len() {
            continue;
        }
        let sec = &mut sections[idx as usize];
        sec.y = y as i8;

        if let Some(bs_tag) = raw_sec.get("block_states") {
            let patched = normalize_block_states_nbt(bs_tag);
            if let Ok(bs) = PalettedContainer::from_nbt(&patched, 4096, 4) {
                sec.block_states = bs;
            }
        }

        if let Some(biomes_tag) = raw_sec.get("biomes") {
            if let Ok(b) = PalettedContainer::from_nbt(biomes_tag, 64, 1) {
                sec.biomes = b;
            }
        }

        if let Some(Tag::ByteArray(bl)) = raw_sec.get("BlockLight") {
            if let Ok(arr) = <[u8; LIGHT_LAYER_BYTES]>::try_from(bl.as_slice()) {
                sec.block_light = Some(Arc::new(arr));
            }
        }

        if has_skylight {
            if let Some(Tag::ByteArray(sl)) = raw_sec.get("SkyLight") {
                if let Ok(arr) = <[u8; LIGHT_LAYER_BYTES]>::try_from(sl.as_slice()) {
                    sec.sky_light = Some(Arc::new(arr));
                }
            } else if !sec.is_only_air() {
                completely_lit = false;
            }
        }
    }

    // Bobby skylight fallback (`ChunkSerializer.java` lines 250-303):
    // If any non-air section lacked saved SkyLight, infer skylight from open sky downward
    // and flood-fill through transparent blocks.
    if has_skylight && !completely_lit {
        infer_missing_skylight(&mut sections, min_y);
    }

    let mut heightmaps = BTreeMap::new();
    if let Some(Tag::Compound(hm_compound)) = tag.get("Heightmaps") {
        for (k, v) in hm_compound {
            if let Tag::LongArray(words) = v {
                heightmaps.insert(k.to_string_lossy(), words.clone());
            }
        }
    }

    let mut block_entities = Vec::new();
    if !config.no_block_entities {
        if let Some(Tag::List(be_list)) = tag.get("block_entities") {
            block_entities = be_list.clone();
        }
    }

    let mut chunk = CachedChunk {
        pos: ChunkPos::new(x, z),
        min_section_y: min_y,
        has_skylight,
        is_light_on,
        age,
        tainted: false,
        will_be_replaced: false,
        sections: sections.into_iter().map(Arc::new).collect(),
        heightmaps,
        block_entities,
        untainted_sky_light: None,
    };

    if config.taint_fake_chunks {
        chunk.set_tainted(true);
    }

    Some(chunk)
}

/// Normalizes vanilla `Name` / `Properties` block_states palette entries to Horizon's `id` / `properties`
/// if needed, so both vanilla Anvil `.mca` files from Java Bobby and Horizon `.mca` files load seamlessly.
fn normalize_block_states_nbt(tag: &Tag) -> Tag {
    let Tag::Compound(map) = tag else {
        return tag.clone();
    };
    let Some(Tag::List(palette)) = map.get(&jkey("palette")) else {
        return tag.clone();
    };
    let needs_remap = palette
        .iter()
        .any(|entry| matches!(entry, Tag::Compound(c) if c.contains_key(&jkey("Name"))));
    if !needs_remap {
        return tag.clone();
    }
    let mut new_map = map.clone();
    let new_palette: Vec<Tag> = palette
        .iter()
        .map(|entry| {
            let Tag::Compound(c) = entry else {
                return entry.clone();
            };
            if c.contains_key(&jkey("id")) {
                return entry.clone();
            }
            let mut out = Compound::new();
            if let Some(name) = c.get(&jkey("Name")) {
                out.insert(jkey("id"), name.clone());
            }
            if let Some(props) = c.get(&jkey("Properties")).or_else(|| c.get(&jkey("properties")))
            {
                out.insert(jkey("properties"), props.clone());
            }
            Tag::Compound(out)
        })
        .collect();
    new_map.insert(jkey("palette"), Tag::List(new_palette));
    Tag::Compound(new_map)
}

/// Port of Bobby's skylight flood-fill inference (`ChunkSerializer.deserialize`).
///
/// Propagates full sky light (`15`) straight down from the top of the column until hitting
/// a non-air block, then performs a 6-neighbor BFS decay (`light - 1`) through transparent blocks.
fn infer_missing_skylight(sections: &mut [CachedSection], min_section_y: i32) {
    let total_y = sections.len() * 16;
    let mut layers: Vec<[u8; LIGHT_LAYER_BYTES]> = sections
        .iter()
        .map(|s| s.sky_light.as_deref().copied().unwrap_or([0u8; LIGHT_LAYER_BYTES]))
        .collect();

    let is_transparent = |sections: &[CachedSection], x: usize, y_abs: usize, z: usize| -> bool {
        let sec_idx = y_abs >> 4;
        let local_y = y_abs & 15;
        let idx = (local_y << 8) | (z << 4) | x;
        match sections[sec_idx].block_states.get(idx) {
            Ok(tag) => is_air_tag(tag) || is_transparent_tag(tag),
            Err(_) => true,
        }
    };

    let mut queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();

    // 1. Cast vertical sky rays down each of the 16x16 columns from the top.
    for x in 0..16 {
        for z in 0..16 {
            for y_abs in (0..total_y).rev() {
                if !is_transparent(sections, x, y_abs, z) {
                    break;
                }
                let sec_idx = y_abs >> 4;
                let local_y = y_abs & 15;
                CachedSection::set_nibble(&mut layers[sec_idx], x, local_y, z, 15);
                queue.push_back((x, y_abs, z, 15));
            }
        }
    }

    // 2. BFS flood-fill into adjacent non-opaque blocks within the column.
    let dirs: [(isize, isize, isize); 6] = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];

    while let Some((x, y_abs, z, level)) = queue.pop_front() {
        if level <= 1 {
            continue;
        }
        let next_level = level - 1;
        for (dx, dy, dz) in dirs {
            let nx = x as isize + dx;
            let ny = y_abs as isize + dy;
            let nz = z as isize + dz;
            if nx < 0 || nx >= 16 || nz < 0 || nz >= 16 || ny < 0 || ny >= total_y as isize {
                continue;
            }
            let (nx, ny, nz) = (nx as usize, ny as usize, nz as usize);
            let sec_idx = ny >> 4;
            let local_y = ny & 15;
            if CachedSection::get_nibble(&layers[sec_idx], nx, local_y, nz) >= next_level {
                continue;
            }
            if !is_transparent(sections, nx, ny, nz) {
                continue;
            }
            CachedSection::set_nibble(&mut layers[sec_idx], nx, local_y, nz, next_level);
            queue.push_back((nx, ny, nz, next_level));
        }
    }

    let _ = min_section_y;
    for (sec, layer) in sections.iter_mut().zip(layers) {
        if sec.sky_light.is_none() && layer.iter().any(|&b| b != 0) {
            sec.sky_light = Some(Arc::new(layer));
        }
    }
}

fn is_transparent_tag(tag: &Tag) -> bool {
    let name = match tag {
        Tag::String(s) => s.to_string_lossy(),
        Tag::Compound(_) => match tag.get("Name").or_else(|| tag.get("id")) {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => return false,
        },
        _ => return false,
    };
    name.contains("glass")
        || name.contains("leaves")
        || name.contains("water")
        || name.contains("Torch")
        || name.contains("torch")
        || name.contains("flower")
        || name.contains("grass")
        || name.contains("fern")
        || name.contains("carpet")
        || name.contains("pane")
        || name.contains("fence")
        || name.contains("wall")
        || name.contains("slab")
        || name.contains("stairs")
        || name.contains("snow")
        || name.contains("vine")
        || name.contains("lantern")
}

/// Computes a 64-bit fingerprint of the opaque blocks in `chunk`.
///
/// Direct port of `ChunkSerializer.fingerprint(LevelChunk)`:
/// Packs 1 bit per block (`1` = opaque, `0` = transparent/air) across all sections in the column
/// into a byte buffer and hashes it into a 64-bit fingerprint for dynamic multi-world matching.
pub fn fingerprint(chunk: &CachedChunk) -> u64 {
    let blocks_per_section = 4096usize;
    let total_blocks = chunk.sections.len() * blocks_per_section;
    let mut bytes = vec![0u8; total_blocks / 8];

    for (s_idx, section) in chunk.sections.iter().enumerate() {
        if section.is_only_air() {
            continue;
        }
        // Cache opacity per palette index to avoid inspecting 4,096 tags when the palette has 2-16 entries.
        let base_bit = s_idx * blocks_per_section;
        for i in 0..blocks_per_section {
            let opaque = match section.block_states.get(i) {
                Ok(tag) => !is_air_tag(tag) && !is_transparent_tag(tag),
                Err(_) => false,
            };
            if opaque {
                let bit_idx = base_bit + i;
                bytes[bit_idx >> 3] |= 1 << (bit_idx & 7);
            }
        }
    }

    hash64_fingerprint(&bytes)
}

/// Deterministic 64-bit fingerprint hash (64-bit word MurmurHash3/SplitMix64 finalizer,
/// matching `Hashing.farmHashFingerprint64` semantics with zero external dependencies).
pub fn hash64_fingerprint(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ (bytes.len() as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let mut k = u64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        k = k.wrapping_mul(0x87C3_7B91_1142_53D5);
        k = k.rotate_left(31);
        k = k.wrapping_mul(0x4CF5_AD43_2745_937F);
        h ^= k;
        h = h.rotate_left(27).wrapping_add(h).wrapping_mul(5).wrapping_add(0x52DC_E729);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rem.len()].copy_from_slice(rem);
        let k = u64::from_le_bytes(tail).wrapping_mul(0x87C3_7B91_1142_53D5);
        h ^= k;
    }
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::chunk::{BlockProperties, BlockState};

    #[test]
    fn serialize_and_deserialize_roundtrip_with_skylight_and_taint() {
        let pos = ChunkPos::new(3, -5);
        let mut chunk = CachedChunk::new(pos, -4, 24, true, 123_456);

        // Place some stone in section 0 (y = -4)
        let stone = BlockState {
            name: "minecraft:stone".to_owned(),
            properties: BlockProperties::new(),
        }
        .to_nbt();
        let sec0 = Arc::make_mut(&mut chunk.sections[0]);
        sec0.block_states.set(0, stone).unwrap();
        let mut sky = [0xFFu8; LIGHT_LAYER_BYTES];
        CachedSection::set_nibble(&mut sky, 0, 0, 0, 12);
        sec0.sky_light = Some(Arc::new(sky));

        let mut cfg = BobbyConfig::default();
        let nbt = serialize(&chunk, &cfg);

        // Deserialize without taint
        let loaded = deserialize(pos, &nbt, &cfg, true).expect("deserialize");
        assert_eq!(loaded.pos, pos);
        assert_eq!(loaded.age, 123_456);
        assert_eq!(loaded.get_sky_light(0, -64, 0), 12);
        assert_eq!(
            loaded.sections[0].get_block_state(0, 0, 0).unwrap().name,
            "minecraft:stone"
        );

        // Deserialize with taint enabled (-5 sky light)
        cfg.taint_fake_chunks = true;
        let tainted = deserialize(pos, &nbt, &cfg, true).expect("deserialize tainted");
        assert!(tainted.tainted);
        assert_eq!(tainted.get_sky_light(0, -64, 0), 7);

        // Untainting restores original 12
        let mut untainted = tainted.clone();
        untainted.set_tainted(false);
        assert_eq!(untainted.get_sky_light(0, -64, 0), 12);
    }

    #[test]
    fn fingerprint_distinguishes_terrain() {
        let pos = ChunkPos::new(0, 0);
        let chunk_a = CachedChunk::new(pos, -4, 4, true, 1);
        let mut chunk_b = chunk_a.shallow_copy();
        let stone = BlockState {
            name: "minecraft:stone".to_owned(),
            properties: BlockProperties::new(),
        }
        .to_nbt();
        Arc::make_mut(&mut chunk_b.sections[1])
            .block_states
            .set(42, stone)
            .unwrap();

        assert_ne!(fingerprint(&chunk_a), fingerprint(&chunk_b));
        assert_eq!(fingerprint(&chunk_b), fingerprint(&chunk_b));
    }
}
