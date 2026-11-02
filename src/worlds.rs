//! Dynamic multi-world detection, fingerprint matching, and region merging (`Worlds`).
//!
//! Direct Rust port of `de.johni0702.minecraft.bobby.Worlds`.
//! When a multiplayer server reuses the same dimension identifier (e.g. `minecraft:overworld`)
//! across multiple lobbies or sub-worlds, `Worlds` compares 64-bit opaque-block fingerprints
//! of live chunks against cached worlds (`worlds.meta` and `r.<x>.<z>.meta`) to automatically
//! select the matching world cache and merge fragmented caches.

use super::storage::FakeChunkStorage;
use super::tracker::RegionPos;
use horizon_core::position::ChunkPos;
use horizon_world::nbt::{self, Compound, JavaString, Limits, NamedTag, Tag};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn jkey(s: &str) -> JavaString {
    JavaString::from(s)
}

/// Metadata for a single region (`r.<rx>.<rz>.meta`), storing chunk coordinates,
/// timestamps (`chunk_ages`), and 64-bit terrain fingerprints (`chunk_fingerprints`).
#[derive(Clone, Debug, Default)]
pub struct RegionMeta {
    pub region: RegionPos,
    /// Map from chunk coordinate (`ChunkPos`) to `(age, fingerprint)`.
    pub chunks: HashMap<ChunkPos, (i64, u64)>,
    dirty: bool,
}

impl RegionMeta {
    pub fn new(region: RegionPos) -> Self {
        Self {
            region,
            chunks: HashMap::new(),
            dirty: false,
        }
    }

    /// Loads `r.<rx>.<rz>.meta` from `directory`, if it exists.
    pub fn load(directory: &Path, region: RegionPos) -> io::Result<Self> {
        let path = directory.join(region.meta_file_name());
        if !path.exists() {
            return Ok(Self::new(region));
        }
        let bytes = fs::read(&path)?;
        let (named, _) = nbt::read_named(&bytes, Limits::saved())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Self::from_nbt(region, &named.tag)
    }

    /// Saves `r.<rx>.<rz>.meta` to `directory` if modified.
    pub fn save(&mut self, directory: &Path) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        fs::create_dir_all(directory)?;
        let path = directory.join(self.region.meta_file_name());
        if self.chunks.is_empty() {
            if path.exists() {
                let _ = fs::remove_file(path);
            }
            self.dirty = false;
            return Ok(());
        }
        let named = NamedTag {
            name: jkey(""),
            tag: self.to_nbt(),
        };
        let bytes = nbt::write_named(&named, Limits::saved())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(path, bytes)?;
        self.dirty = false;
        Ok(())
    }

    pub fn put(&mut self, pos: ChunkPos, age: i64, fingerprint: u64) {
        self.chunks.insert(pos, (age, fingerprint));
        self.dirty = true;
    }

    pub fn get(&self, pos: ChunkPos) -> Option<(i64, u64)> {
        self.chunks.get(&pos).copied()
    }

    fn to_nbt(&self) -> Tag {
        let mut entries: Vec<(ChunkPos, i64, u64)> = self
            .chunks
            .iter()
            .map(|(&pos, &(age, fp))| (pos, age, fp))
            .collect();
        entries.sort_by_key(|(pos, _, _)| (pos.x, pos.z));

        let mut coords = Vec::with_capacity(entries.len() * 2);
        let mut ages = Vec::with_capacity(entries.len());
        let mut fingerprints = Vec::with_capacity(entries.len());

        for (pos, age, fp) in entries {
            coords.push(pos.x);
            coords.push(pos.z);
            ages.push(age);
            fingerprints.push(fp as i64);
        }

        let mut map = Compound::new();
        map.insert(jkey("chunk_coords"), Tag::IntArray(coords));
        map.insert(jkey("chunk_ages"), Tag::LongArray(ages));
        map.insert(jkey("chunk_fingerprints"), Tag::LongArray(fingerprints));
        Tag::Compound(map)
    }

    fn from_nbt(region: RegionPos, tag: &Tag) -> io::Result<Self> {
        let Some(Tag::IntArray(coords)) = tag.get("chunk_coords") else {
            return Ok(Self::new(region));
        };
        let Some(Tag::LongArray(ages)) = tag.get("chunk_ages") else {
            return Ok(Self::new(region));
        };
        let Some(Tag::LongArray(fps)) = tag.get("chunk_fingerprints") else {
            return Ok(Self::new(region));
        };
        let count = (coords.len() / 2).min(ages.len()).min(fps.len());
        let mut chunks = HashMap::with_capacity(count);
        for i in 0..count {
            let pos = ChunkPos::new(coords[i * 2], coords[i * 2 + 1]);
            chunks.insert(pos, (ages[i], fps[i] as u64));
        }
        Ok(Self {
            region,
            chunks,
            dirty: false,
        })
    }
}

/// Information about a single sub-world managed by [`Worlds`].
#[derive(Clone, Debug)]
pub struct WorldEntry {
    pub id: u32,
    pub title: String,
    pub directory: PathBuf,
    pub storage: Arc<FakeChunkStorage>,
    pub regions: HashMap<RegionPos, RegionMeta>,
}

/// Match statistics for a candidate world against currently observed live chunks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldMatchScore {
    pub matching: usize,
    pub mismatching: usize,
    pub unknown: usize,
}

/// Dynamic multi-world manager (`de.johni0702.minecraft.bobby.Worlds`).
pub struct Worlds {
    base_directory: PathBuf,
    next_id: u32,
    worlds: BTreeMap<u32, WorldEntry>,
    current_world_id: Option<u32>,
    /// Live chunks observed during the current session: `ChunkPos -> (age, fingerprint)`.
    observed_chunks: HashMap<ChunkPos, (i64, u64)>,
}

impl Worlds {
    pub const META_FILE_NAME: &'static str = "worlds.meta";

    /// Opens or initializes the `Worlds` manager for `base_directory`.
    pub fn open(base_directory: impl AsRef<Path>) -> io::Result<Self> {
        let base_directory = base_directory.as_ref().to_path_buf();
        fs::create_dir_all(&base_directory)?;

        let meta_path = base_directory.join(Self::META_FILE_NAME);
        let mut next_id = 1u32;
        let mut titles: BTreeMap<u32, String> = BTreeMap::new();

        if meta_path.exists() {
            if let Ok(bytes) = fs::read(&meta_path) {
                if let Ok((named, _)) = nbt::read_named(&bytes, Limits::saved()) {
                    if let Some(Tag::Int(id)) = named.tag.get("next_id") {
                        next_id = (*id).max(1) as u32;
                    }
                    if let Some(Tag::Compound(worlds_map)) = named.tag.get("worlds") {
                        for (k, v) in worlds_map {
                            if let (Ok(id), Tag::String(title)) =
                                (k.to_string_lossy().parse::<u32>(), v)
                            {
                                titles.insert(id, title.to_string_lossy());
                                next_id = next_id.max(id + 1);
                            }
                        }
                    }
                }
            }
        }

        // Always ensure world 0 ("default") exists if there are existing region files or no worlds yet.
        if titles.is_empty() {
            titles.insert(0, "default".to_owned());
        }

        let mut worlds = BTreeMap::new();
        for (id, title) in titles {
            let dir = if id == 0 {
                base_directory.clone()
            } else {
                base_directory.join(id.to_string())
            };
            let storage = FakeChunkStorage::open(&dir, true)?;
            let mut regions = HashMap::new();
            if let Ok(region_list) = storage.list_regions() {
                for r in region_list {
                    if let Ok(meta) = RegionMeta::load(&dir, r) {
                        regions.insert(r, meta);
                    }
                }
            }
            worlds.insert(
                id,
                WorldEntry {
                    id,
                    title,
                    directory: dir,
                    storage,
                    regions,
                },
            );
        }

        let current_world_id = worlds.keys().next().copied();

        Ok(Self {
            base_directory,
            next_id,
            worlds,
            current_world_id,
            observed_chunks: HashMap::new(),
        })
    }

    pub fn current_world_id(&self) -> Option<u32> {
        self.current_world_id
    }

    pub fn current_world(&self) -> Option<&WorldEntry> {
        self.current_world_id.and_then(|id| self.worlds.get(&id))
    }

    pub fn worlds(&self) -> &BTreeMap<u32, WorldEntry> {
        &self.worlds
    }

    /// Creates a new sub-world cache with the given `title` and makes it active (`/bobby create`).
    pub fn create_world(&mut self, title: impl Into<String>) -> io::Result<u32> {
        let id = self.next_id;
        self.next_id += 1;
        let title = title.into();
        let dir = self.base_directory.join(id.to_string());
        let storage = FakeChunkStorage::open(&dir, true)?;
        let mut entry = WorldEntry {
            id,
            title,
            directory: dir,
            storage,
            regions: HashMap::new(),
        };
        // Seed the new world's metadata with currently observed chunks.
        for (&pos, &(age, fp)) in &self.observed_chunks {
            let region = RegionPos::from_chunk_pos(pos);
            entry
                .regions
                .entry(region)
                .or_insert_with(|| RegionMeta::new(region))
                .put(pos, age, fp);
        }
        self.worlds.insert(id, entry);
        self.current_world_id = Some(id);
        self.save_meta()?;
        Ok(id)
    }

    /// Records a newly computed 64-bit opaque-block fingerprint for a live server chunk (`pos`),
    /// updates the active world's region metadata, and automatically switches worlds if
    /// `dynamic_multi_world` detects a better match.
    ///
    /// Returns `Some(new_world_id)` if the active world changed.
    pub fn observe_chunk(
        &mut self,
        pos: ChunkPos,
        age: i64,
        fingerprint: u64,
        auto_switch: bool,
    ) -> Option<u32> {
        self.observed_chunks.insert(pos, (age, fingerprint));

        let mut switched = None;
        if auto_switch {
            let best = self.find_best_matching_world();
            if best != self.current_world_id && best.is_some() {
                self.current_world_id = best;
                switched = best;
            }
        }

        if let Some(curr_id) = self.current_world_id {
            if let Some(world) = self.worlds.get_mut(&curr_id) {
                let region = RegionPos::from_chunk_pos(pos);
                let meta = world
                    .regions
                    .entry(region)
                    .or_insert_with(|| RegionMeta::new(region));
                meta.put(pos, age, fingerprint);
            }
        }

        switched
    }

    /// Computes `(matching, mismatching, unknown)` fingerprint counts for `world_id`
    /// against the currently observed live chunks (`/bobby worlds`).
    pub fn score_world(&self, world_id: u32) -> WorldMatchScore {
        let Some(world) = self.worlds.get(&world_id) else {
            return WorldMatchScore::default();
        };
        let mut score = WorldMatchScore::default();
        for (&pos, &(_, live_fp)) in &self.observed_chunks {
            let region = RegionPos::from_chunk_pos(pos);
            match world.regions.get(&region).and_then(|r| r.get(pos)) {
                Some((_, saved_fp)) if saved_fp == live_fp => score.matching += 1,
                Some(_) => score.mismatching += 1,
                None => score.unknown += 1,
            }
        }
        score
    }

    /// Finds the world ID with the highest number of matching chunk fingerprints
    /// and a low mismatch ratio (`matching > mismatching * 2`).
    pub fn find_best_matching_world(&self) -> Option<u32> {
        let mut best_id = self.current_world_id;
        let mut best_matches = 0usize;

        for &id in self.worlds.keys() {
            let s = self.score_world(id);
            if s.matching > s.mismatching * 2 && s.matching > best_matches {
                best_matches = s.matching;
                best_id = Some(id);
            }
        }
        best_id
    }

    /// Merges chunks from `source_id` into `target_id` (`/bobby merge`), copying newer chunks
    /// (`source_age >= target_age`) and removing `source_id` once merged.
    pub fn merge_worlds(&mut self, source_id: u32, target_id: u32) -> io::Result<usize> {
        if source_id == target_id {
            return Ok(0);
        }
        let Some(source) = self.worlds.remove(&source_id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("source world {source_id} not found"),
            ));
        };
        let Some(target) = self.worlds.get_mut(&target_id) else {
            self.worlds.insert(source_id, source);
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("target world {target_id} not found"),
            ));
        };

        let mut merged_chunks = 0usize;
        let mut all_regions: HashSet<RegionPos> = source.regions.keys().copied().collect();
        if let Ok(disk_regions) = source.storage.list_regions() {
            all_regions.extend(disk_regions);
        }

        for region in all_regions {
            let chunks = source.storage.region_chunks(region).unwrap_or_default();
            for pos in chunks {
                let src_meta = source.regions.get(&region).and_then(|m| m.get(pos));
                let dst_meta = target.regions.get(&region).and_then(|m| m.get(pos));

                let should_copy = match (src_meta, dst_meta) {
                    (Some((src_age, _)), Some((dst_age, _))) => src_age >= dst_age,
                    _ => true,
                };
                if should_copy {
                    if let Ok(Some((nbt, _))) = source.storage.load_tag(pos) {
                        target.storage.write_chunk(pos, &nbt)?;
                        if let Some((age, fp)) = src_meta {
                            target
                                .regions
                                .entry(region)
                                .or_insert_with(|| RegionMeta::new(region))
                                .put(pos, age, fp);
                        }
                        merged_chunks += 1;
                    }
                }
            }
        }

        let _ = source.storage.close();
        if source_id != 0 && source.directory.exists() {
            let _ = fs::remove_dir_all(&source.directory);
        }
        if self.current_world_id == Some(source_id) {
            self.current_world_id = Some(target_id);
        }
        self.save_meta()?;
        Ok(merged_chunks)
    }

    /// Saves `worlds.meta` and all modified `r.<x>.<z>.meta` files to disk.
    pub fn save_all(&mut self) -> io::Result<()> {
        for world in self.worlds.values_mut() {
            for meta in world.regions.values_mut() {
                meta.save(&world.directory)?;
            }
            world.storage.close()?;
        }
        self.save_meta()
    }

    fn save_meta(&self) -> io::Result<()> {
        let mut worlds_map = Compound::new();
        for (&id, entry) in &self.worlds {
            worlds_map.insert(
                jkey(&id.to_string()),
                Tag::String(jkey(&entry.title)),
            );
        }
        let mut root = Compound::new();
        root.insert(jkey("next_id"), Tag::Int(self.next_id as i32));
        root.insert(jkey("worlds"), Tag::Compound(worlds_map));

        let named = NamedTag {
            name: jkey(""),
            tag: Tag::Compound(root),
        };
        let bytes = nbt::write_named(&named, Limits::saved())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(self.base_directory.join(Self::META_FILE_NAME), bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_WORLD_DIR: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn multi_world_matching_and_merging() {
        let id = NEXT_WORLD_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "horizon-bobby-worlds-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let mut worlds = Worlds::open(&dir).unwrap();
        assert_eq!(worlds.current_world_id(), Some(0));

        // Observe two chunks in world 0
        worlds.observe_chunk(ChunkPos::new(1, 1), 100, 0xAAAA_1111, false);
        worlds.observe_chunk(ChunkPos::new(1, 2), 100, 0xAAAA_2222, false);

        // Create a second world ("lobby-2") with different terrain at (1, 1) and (1, 2)
        worlds.observed_chunks.clear();
        let w1 = worlds.create_world("lobby-2").unwrap();
        assert_eq!(worlds.current_world_id(), Some(w1));
        worlds.observe_chunk(ChunkPos::new(1, 1), 200, 0xBBBB_1111, false);
        worlds.observe_chunk(ChunkPos::new(1, 2), 200, 0xBBBB_2222, false);

        // Now simulate re-entering world 0 and observing its chunks with auto_switch = true
        worlds.observed_chunks.clear();
        let switched = worlds.observe_chunk(ChunkPos::new(1, 1), 300, 0xAAAA_1111, true);
        assert_eq!(switched, Some(0));
        assert_eq!(worlds.current_world_id(), Some(0));

        worlds.save_all().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}

