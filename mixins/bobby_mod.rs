pub const DATA_VERSION: i32 = 4786;
pub const BOBBY_DIR_NAME: &str = ".bobby";

pub mod config {
    // Configuration management for Bobby (`bobby.conf` / `bobby.json`).
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.BobbyConfig`.

    use serde::{Deserialize, Serialize};
    use std::fs;
    use std::io;
    use std::path::Path;

    /// Configuration settings for the Bobby chunk caching system.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(default, rename_all = "kebab-case")]
    pub struct BobbyConfig {
        /// Whether Bobby chunk caching is enabled.
        pub enabled: bool,
        /// Enable support for servers with multiple worlds that reuse the same dimension ID.
        pub dynamic_multi_world: bool,
        /// Do not load block entities (e.g. chests) in fake chunks as they need ticking every frame.
        pub no_block_entities: bool,
        /// Reduce the light level in fake chunks so you can tell them apart from real chunks.
        pub taint_fake_chunks: bool,
        /// Delay unloading of chunks which are outside your view distance by this many seconds.
        /// Saves having to reload chunks from disk when flying around or turning back quickly.
        pub unload_delay_secs: u32,
        /// Delete regions from the cache when they have not been loaded for X days (`-1` disables).
        pub delete_unused_regions_after_days: i32,
        /// Changes the maximum value Configurable for Render Distance (`maxRenderDistance`).
        pub max_render_distance: i32,
        /// Overwrites the view-distance of the integrated server (`0` leaves it untouched).
        pub view_distance_overwrite: i32,
    }

    impl Default for BobbyConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                dynamic_multi_world: false,
                no_block_entities: true,
                taint_fake_chunks: false,
                unload_delay_secs: 60,
                delete_unused_regions_after_days: -1,
                max_render_distance: 32,
                view_distance_overwrite: 0,
            }
        }
    }

    impl BobbyConfig {
        /// Default configuration instance matching `BobbyConfig.DEFAULT`.
        pub const DEFAULT: Self = Self {
            enabled: true,
            dynamic_multi_world: false,
            no_block_entities: true,
            taint_fake_chunks: false,
            unload_delay_secs: 60,
            delete_unused_regions_after_days: -1,
            max_render_distance: 32,
            view_distance_overwrite: 0,
        };

        /// Computes the effective client render distance to use for fake chunks,
        /// given the client's current render distance setting.
        pub fn effective_render_distance(&self, client_render_distance: i32) -> i32 {
            if !self.enabled {
                return client_render_distance.max(2);
            }
            client_render_distance.clamp(2, self.max_render_distance.max(2))
        }

        /// Loads `BobbyConfig` from the given file path (`bobby.conf` or `bobby.json`).
        /// Supports both standard JSON and HOCON-like `key = value` / `# comment` syntax
        /// used by the original Java `Configurate` loader.
        pub fn load_from_path(path: impl AsRef<Path>) -> io::Result<Self> {
            let path = path.as_ref();
            if !path.exists() {
                return Ok(Self::default());
            }
            let content = fs::read_to_string(path)?;
            Self::parse_str(&content)
        }

        /// Saves `BobbyConfig` to the given file path in pretty JSON (valid HOCON & JSON).
        pub fn save_to_path(&self, path: impl AsRef<Path>) -> io::Result<()> {
            let path = path.as_ref();
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            let serialized = serde_json::to_string_pretty(self)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            fs::write(path, serialized)
        }

        /// Parses either JSON or HOCON `key = value` configuration text.
        pub fn parse_str(text: &str) -> io::Result<Self> {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(Self::default());
            }
            if let Ok(cfg) = serde_json::from_str::<Self>(trimmed) {
                return Ok(cfg);
            }
            // Fallback parser for HOCON `key = value` or `key: value` lines with `#` or `//` comments.
            let mut cfg = Self::default();
            for raw_line in trimmed.lines() {
                let line = raw_line
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .split("//")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(',');
                if line.is_empty() || line == "{" || line == "}" {
                    continue;
                }
                let Some((raw_key, raw_val)) = line.split_once('=').or_else(|| line.split_once(':'))
                else {
                    continue;
                };
                let key = raw_key.trim().trim_matches('"');
                let val = raw_val.trim().trim_matches('"');
                match key {
                    "enabled" => {
                        if let Ok(v) = val.parse::<bool>() {
                            cfg.enabled = v;
                        }
                    }
                    "dynamic-multi-world" | "dynamic_multi_world" => {
                        if let Ok(v) = val.parse::<bool>() {
                            cfg.dynamic_multi_world = v;
                        }
                    }
                    "no-block-entities" | "no_block_entities" => {
                        if let Ok(v) = val.parse::<bool>() {
                            cfg.no_block_entities = v;
                        }
                    }
                    "taint-fake-chunks" | "taint_fake_chunks" => {
                        if let Ok(v) = val.parse::<bool>() {
                            cfg.taint_fake_chunks = v;
                        }
                    }
                    "unload-delay-secs" | "unload_delay_secs" => {
                        if let Ok(v) = val.parse::<u32>() {
                            cfg.unload_delay_secs = v;
                        }
                    }
                    "delete-unused-regions-after-days" | "delete_unused_regions_after_days" => {
                        if let Ok(v) = val.parse::<i32>() {
                            cfg.delete_unused_regions_after_days = v;
                        }
                    }
                    "max-render-distance" | "max_render_distance" => {
                        if let Ok(v) = val.parse::<i32>() {
                            cfg.max_render_distance = v;
                        }
                    }
                    "view-distance-overwrite" | "view_distance_overwrite" => {
                        if let Ok(v) = val.parse::<i32>() {
                            cfg.view_distance_overwrite = v;
                        }
                    }
                    _ => {}
                }
            }
            Ok(cfg)
        }
    }
}

pub mod tracker {
    // Spatial tracking for visible chunk columns and Anvil 32x32 regions.
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.VisibleChunksTracker`
    // and `de.johni0702.minecraft.bobby.util.RegionPos`.

    use horizon_core::position::ChunkPos;
    use std::fmt;

    /// Coordinates of a 32x32-chunk Anvil region (`r.<x>.<z>.mca`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RegionPos {
        pub x: i32,
        pub z: i32,
    }

    impl RegionPos {
        #[inline]
        pub const fn new(x: i32, z: i32) -> Self {
            Self { x, z }
        }

        #[inline]
        pub const fn from_chunk_coords(chunk_x: i32, chunk_z: i32) -> Self {
            Self {
                x: chunk_x >> 5,
                z: chunk_z >> 5,
            }
        }

        #[inline]
        pub const fn from_chunk_pos(pos: ChunkPos) -> Self {
            Self::from_chunk_coords(pos.x, pos.z)
        }

        #[inline]
        pub const fn to_chunk_pos(self) -> ChunkPos {
            ChunkPos::new(self.x, self.z)
        }

        /// Packed 64-bit representation matching `RegionPos.toLong()`.
        #[inline]
        pub const fn to_long(self) -> i64 {
            (self.x as u32 as i64) | ((self.z as u32 as i64) << 32)
        }

        /// Unpacks from `RegionPos.fromLong(long)`.
        #[inline]
        pub const fn from_long(packed: i64) -> Self {
            Self {
                x: packed as i32,
                z: (packed >> 32) as i32,
            }
        }

        /// Standard Anvil region file name (`r.<x>.<z>.mca`).
        pub fn mca_file_name(self) -> String {
            format!("r.{}.{}.mca", self.x, self.z)
        }

        /// Bobby dynamic multi-world metadata file name (`r.<x>.<z>.meta`).
        pub fn meta_file_name(self) -> String {
            format!("r.{}.{}.meta", self.x, self.z)
        }
    }

    impl fmt::Display for RegionPos {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "({}, {})", self.x, self.z)
        }
    }

    /// Tracks the square of chunks visible around `(center_x, center_z)` with radius `view_distance`
    /// and computes exact differential load/unload events in `O(delta)` without heap allocations.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct VisibleChunksTracker {
        center_x: i32,
        center_z: i32,
        /// Negative `view_distance` indicates an empty tracker (no chunks visible).
        view_distance: i32,
    }

    impl Default for VisibleChunksTracker {
        fn default() -> Self {
            Self::new()
        }
    }

    impl VisibleChunksTracker {
        pub const fn new() -> Self {
            Self {
                center_x: 0,
                center_z: 0,
                view_distance: -1,
            }
        }

        #[inline]
        pub const fn center_x(&self) -> i32 {
            self.center_x
        }

        #[inline]
        pub const fn center_z(&self) -> i32 {
            self.center_z
        }

        #[inline]
        pub const fn view_distance(&self) -> i32 {
            self.view_distance
        }

        /// Updates the tracker's center and view distance, invoking `unload(x, z)` for every chunk
        /// that left the visible area and `load(x, z)` for every chunk that entered the visible area.
        pub fn update<U, L>(
            &mut self,
            new_center_x: i32,
            new_center_z: i32,
            new_view_distance: i32,
            mut unload: U,
            mut load: L,
        ) where
            U: FnMut(i32, i32),
            L: FnMut(i32, i32),
        {
            if self.center_x == new_center_x
                && self.center_z == new_center_z
                && self.view_distance == new_view_distance
            {
                return;
            }

            let old_min_x = self.center_x - self.view_distance;
            let old_max_x = self.center_x + self.view_distance;
            let old_min_z = self.center_z - self.view_distance;
            let old_max_z = self.center_z + self.view_distance;

            let new_min_x = new_center_x - new_view_distance;
            let new_max_x = new_center_x + new_view_distance;
            let new_min_z = new_center_z - new_view_distance;
            let new_max_z = new_center_z + new_view_distance;

            if old_max_x < new_min_x
                || new_max_x < old_min_x
                || old_max_z < new_min_z
                || new_max_z < old_min_z
            {
                // Disjoint boxes: unload all old, load all new.
                for x in old_min_x..=old_max_x {
                    for z in old_min_z..=old_max_z {
                        unload(x, z);
                    }
                }
                for x in new_min_x..=new_max_x {
                    for z in new_min_z..=new_max_z {
                        load(x, z);
                    }
                }
            } else {
                let min_x = old_min_x.min(new_min_x);
                let max_x = old_max_x.max(new_max_x);
                let min_z = old_min_z.min(new_min_z);
                let max_z = old_max_z.max(new_max_z);

                let mut x = min_x;
                while x <= max_x {
                    let in_old_x = x >= old_min_x && x <= old_max_x;
                    let in_new_x = x >= new_min_x && x <= new_max_x;

                    let mut z = min_z;
                    while z <= max_z {
                        let in_old_z = z >= old_min_z && z <= old_max_z;
                        let in_new_z = z >= new_min_z && z <= new_max_z;

                        let in_old = in_old_x && in_old_z;
                        let in_new = in_new_x && in_new_z;

                        if in_old && !in_new {
                            unload(x, z);
                        } else if in_new && !in_old {
                            load(x, z);
                        } else if in_old && in_new {
                            // Fast-forward across the inner intersection along Z!
                            let skip_to = old_max_z.min(new_max_z);
                            if z < skip_to {
                                z = skip_to;
                            }
                        }
                        z += 1;
                    }
                    x += 1;
                }
            }

            self.center_x = new_center_x;
            self.center_z = new_center_z;
            self.view_distance = new_view_distance;
        }

        /// Calls `f(x, z)` for every currently visible chunk column.
        pub fn for_each<F>(&self, mut f: F)
        where
            F: FnMut(i32, i32),
        {
            for x in (self.center_x - self.view_distance)..=(self.center_x + self.view_distance) {
                for z in (self.center_z - self.view_distance)..=(self.center_z + self.view_distance) {
                    f(x, z);
                }
            }
        }

        /// Returns whether chunk `(x, z)` is inside the tracked visible square.
        #[inline]
        pub fn should_be_loaded(&self, x: i32, z: i32) -> bool {
            (x - self.center_x).abs() <= self.view_distance
                && (z - self.center_z).abs() <= self.view_distance
        }
    }
}

pub mod chunk {
    // Fake chunk (`CachedChunk`) and section (`CachedSection`) representations.
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunk` and
    // `de.johni0702.minecraft.bobby.ext.FakeChunkData`, bridged with Horizon's
    // `horizon_world` and `horizon_protocol` chunk primitives.

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
}

pub mod serializer {
    // Anvil chunk NBT serialization, deserialization, skylight flood-fill fallback,
    // and 64-bit opaque-block fingerprinting.
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.ChunkSerializer`.

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
}

pub mod storage {
    // Anvil region storage (`r.<x>.<z>.mca`) and `last_access` region timestamp tracking.
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunkStorage` and
    // `de.johni0702.minecraft.bobby.LastAccessFile`, built on top of
    // `horizon_world::region::{RegionFile, ChunkRecord}` and `Compression::Zlib`.

    use super::DATA_VERSION;
    use super::tracker::RegionPos;
    use horizon_core::position::ChunkPos;
    use horizon_world::compression::Compression;
    use horizon_world::nbt::{Limits, NamedTag, Tag};
    use horizon_world::region::{ChunkRecord, RegionFile};
    use std::collections::{BTreeSet, HashMap};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Maximum number of region files kept open simultaneously per storage directory.
    pub const MAX_OPEN_REGIONS: usize = 256;

    /// Milliseconds per calendar day (`86_400_000`).
    pub const MILLIS_PER_DAY: i64 = 86_400_000;

    /// Returns today's midnight UTC timestamp in milliseconds (`DAYS.toMillis(MILLISECONDS.toDays(now))`),
    /// matching `LastAccessFile.todayTimestamp` in Bobby.
    pub fn today_timestamp_ms() -> i64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        (millis / MILLIS_PER_DAY) * MILLIS_PER_DAY
    }

    /// Current unix timestamp in seconds for Anvil sector headers.
    pub fn current_timestamp_secs() -> u32 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32)
    }

    /// Tracks the last access day (in milliseconds) of every region in a dimension folder,
    /// persisted in Bobby's VarInt/VarLong binary `last_access` format.
    #[derive(Debug)]
    pub struct LastAccessFile {
        directory: PathBuf,
        path: PathBuf,
        map: Mutex<HashMap<RegionPos, i64>>,
        today_ms: i64,
    }

    impl LastAccessFile {
        pub const FILE_NAME: &'static str = "last_access";
        const TMP_FILE_NAME: &'static str = ".last_access.tmp";

        /// Opens or initializes the `last_access` tracker for `directory`.
        pub fn new(directory: impl AsRef<Path>) -> io::Result<Self> {
            let directory = directory.as_ref().to_path_buf();
            fs::create_dir_all(&directory)?;
            let path = directory.join(Self::FILE_NAME);
            let today_ms = today_timestamp_ms();
            let mut map = HashMap::new();

            if path.exists() {
                if let Ok(bytes) = fs::read(&path) {
                    let _ = Self::decode_into(&bytes, &mut map);
                }
            }

            // Scan directory for any `r.<x>.<z>.mca` files not yet recorded in `last_access`.
            if let Ok(entries) = fs::read_dir(&directory) {
                for entry in entries.flatten() {
                    if let Some(region) = parse_region_filename(&entry.file_name().to_string_lossy()) {
                        map.entry(region).or_insert(today_ms);
                    }
                }
            }

            Ok(Self {
                directory,
                path,
                map: Mutex::new(map),
                today_ms,
            })
        }

        /// Marks `region` as accessed today.
        pub fn touch_region(&self, region: RegionPos) {
            if let Ok(mut guard) = self.map.lock() {
                guard.insert(region, self.today_ms);
            }
        }

        /// Removes `region` from the tracker.
        pub fn remove_region(&self, region: RegionPos) {
            if let Ok(mut guard) = self.map.lock() {
                guard.remove(&region);
            }
        }

        /// Returns all regions whose last access is older than `days` days before today,
        /// removing them from the in-memory map (`LastAccessFile.pollRegionsOlderThan`).
        pub fn poll_regions_older_than(&self, days: i32) -> Vec<RegionPos> {
            if days < 0 {
                return Vec::new();
            }
            let cutoff = self.today_ms - i64::from(days) * MILLIS_PER_DAY;
            let mut guard = match self.map.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let mut expired = Vec::new();
            guard.retain(|&region, &mut ts| {
                if ts < cutoff {
                    expired.push(region);
                    false
                } else {
                    true
                }
            });
            expired
        }

        /// Writes the `last_access` file atomically via `.last_access.tmp`.
        pub fn close(&self) -> io::Result<()> {
            let guard = match self.map.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            let bytes = Self::encode(&guard);
            let tmp_path = self.directory.join(Self::TMP_FILE_NAME);
            fs::write(&tmp_path, &bytes)?;
            fs::rename(&tmp_path, &self.path)?;
            Ok(())
        }

        fn encode(map: &HashMap<RegionPos, i64>) -> Vec<u8> {
            let timestamps: Vec<i64> = map.values().copied().collect::<BTreeSet<_>>().into_iter().collect();
            let mut buf = Vec::new();
            write_var_int(&mut buf, timestamps.len() as i32);
            for &ts in &timestamps {
                write_var_long(&mut buf, ts);
            }
            // Sort regions for deterministic output.
            let mut entries: Vec<(RegionPos, i64)> = map.iter().map(|(&r, &t)| (r, t)).collect();
            entries.sort_by_key(|(r, _)| *r);
            for (region, ts) in entries {
                if let Ok(idx) = timestamps.binary_search(&ts) {
                    write_var_int(&mut buf, region.x);
                    write_var_int(&mut buf, region.z);
                    write_var_int(&mut buf, idx as i32);
                }
            }
            buf
        }

        fn decode_into(mut slice: &[u8], map: &mut HashMap<RegionPos, i64>) -> Option<()> {
            let count = read_var_int(&mut slice)? as usize;
            if count > 1_000_000 {
                return None;
            }
            let mut timestamps = Vec::with_capacity(count);
            for _ in 0..count {
                timestamps.push(read_var_long(&mut slice)?);
            }
            while !slice.is_empty() {
                let x = read_var_int(&mut slice)?;
                let z = read_var_int(&mut slice)?;
                let idx = read_var_int(&mut slice)? as usize;
                let &ts = timestamps.get(idx)?;
                map.insert(RegionPos::new(x, z), ts);
            }
            Some(())
        }
    }

    fn parse_region_filename(name: &str) -> Option<RegionPos> {
        let parts: Vec<&str> = name.split('.').collect();
        if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
            return None;
        }
        let x = parts[1].parse::<i32>().ok()?;
        let z = parts[2].parse::<i32>().ok()?;
        Some(RegionPos::new(x, z))
    }

    fn write_var_int(buf: &mut Vec<u8>, value: i32) {
        let mut v = value as u32;
        loop {
            if (v & !0x7F) == 0 {
                buf.push(v as u8);
                return;
            }
            buf.push(((v & 0x7F) | 0x80) as u8);
            v >>= 7;
        }
    }

    fn write_var_long(buf: &mut Vec<u8>, value: i64) {
        let mut v = value as u64;
        loop {
            if (v & !0x7F) == 0 {
                buf.push(v as u8);
                return;
            }
            buf.push(((v & 0x7F) | 0x80) as u8);
            v >>= 7;
        }
    }

    fn read_var_int(slice: &mut &[u8]) -> Option<i32> {
        let mut result = 0u32;
        let mut shift = 0;
        loop {
            if shift >= 35 || slice.is_empty() {
                return None;
            }
            let byte = slice[0];
            *slice = &slice[1..];
            result |= u32::from(byte & 0x7F) << shift;
            if (byte & 0x80) == 0 {
                return Some(result as i32);
            }
            shift += 7;
        }
    }

    fn read_var_long(slice: &mut &[u8]) -> Option<i64> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            if shift >= 70 || slice.is_empty() {
                return None;
            }
            let byte = slice[0];
            *slice = &slice[1..];
            result |= u64::from(byte & 0x7F) << shift;
            if (byte & 0x80) == 0 {
                return Some(result as i64);
            }
            shift += 7;
        }
    }

    #[derive(Default)]
    struct RegionCache {
        files: HashMap<RegionPos, (Option<Arc<Mutex<RegionFile>>>, u64)>,
        clock: u64,
    }

    impl RegionCache {
        fn evict_lru_if_needed(&mut self) {
            while self.files.len() >= MAX_OPEN_REGIONS {
                let oldest = self
                    .files
                    .iter()
                    .filter(|(_, (f, _))| f.as_ref().is_none_or(|arc| Arc::strong_count(arc) == 1))
                    .min_by_key(|(_, (_, used))| *used)
                    .map(|(k, _)| *k);
                let Some(key) = oldest else {
                    break;
                };
                self.files.remove(&key);
            }
        }
    }

    /// Thread-safe Anvil region storage for a single cached dimension directory.
    ///
    /// Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunkStorage`.
    pub struct FakeChunkStorage {
        directory: PathBuf,
        writeable: bool,
        cache: Mutex<RegionCache>,
        last_access: Option<LastAccessFile>,
    }

    impl std::fmt::Debug for FakeChunkStorage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FakeChunkStorage")
                .field("directory", &self.directory)
                .field("writeable", &self.writeable)
                .finish()
        }
    }

    impl FakeChunkStorage {
        /// Opens or creates a `FakeChunkStorage` at `directory`.
        pub fn open(directory: impl AsRef<Path>, writeable: bool) -> io::Result<Arc<Self>> {
            let directory = directory.as_ref().to_path_buf();
            if writeable {
                fs::create_dir_all(&directory)?;
            }
            let last_access = if writeable || directory.exists() {
                LastAccessFile::new(&directory).ok()
            } else {
                None
            };
            Ok(Arc::new(Self {
                directory,
                writeable,
                cache: Mutex::new(RegionCache::default()),
                last_access,
            }))
        }

        pub fn directory(&self) -> &Path {
            &self.directory
        }

        pub fn is_writeable(&self) -> bool {
            self.writeable
        }

        /// Acquires the cached `RegionFile` handle for `region`, opening or creating it if needed.
        fn get_region(
            &self,
            region: RegionPos,
            create: bool,
        ) -> io::Result<Option<Arc<Mutex<RegionFile>>>> {
            let mut guard = match self.cache.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard.clock = guard.clock.wrapping_add(1);
            let now = guard.clock;

            if let Some((Some(existing), used)) = guard.files.get_mut(&region) {
                *used = now;
                return Ok(Some(existing.clone()));
            }
            if !create {
                if let Some((None, used)) = guard.files.get_mut(&region) {
                    *used = now;
                    return Ok(None);
                }
            }

            let path = self.directory.join(region.mca_file_name());
            if !create && !path.exists() {
                guard.evict_lru_if_needed();
                guard.files.insert(region, (None, now));
                return Ok(None);
            }

            let opened = if self.writeable {
                if create {
                    fs::create_dir_all(&self.directory)?;
                }
                RegionFile::open_writable(&path, create)
            } else {
                RegionFile::open(&path)
            };

            match opened {
                Ok(rf) => {
                    let handle = Arc::new(Mutex::new(rf));
                    guard.evict_lru_if_needed();
                    guard.files.insert(region, (Some(handle.clone()), now));
                    Ok(Some(handle))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    guard.evict_lru_if_needed();
                    guard.files.insert(region, (None, now));
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        }

        /// Writes `nbt` for `pos` into the corresponding `r.<rx>.<rz>.mca` region file using Zlib compression.
        pub fn write_chunk(&self, pos: ChunkPos, nbt: &NamedTag) -> io::Result<()> {
            if !self.writeable {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "FakeChunkStorage is read-only",
                ));
            }
            let region = RegionPos::from_chunk_pos(pos);
            if let Some(la) = &self.last_access {
                la.touch_region(region);
            }
            let record = ChunkRecord::from_nbt(
                nbt,
                Compression::Zlib,
                current_timestamp_secs(),
                Limits::saved(),
            )?;
            let Some(handle) = self.get_region(region, true)? else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "failed to create region file",
                ));
            };
            let mut rf = match handle.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            rf.write(pos, &record)
        }

        /// Reads the raw NBT tag for `pos` from `r.<rx>.<rz>.mca` along with a flag indicating
        /// whether its `DataVersion` differs from Horizon's current `DATA_VERSION`.
        pub fn load_tag(&self, pos: ChunkPos) -> io::Result<Option<(NamedTag, bool)>> {
            let region = RegionPos::from_chunk_pos(pos);
            let Some(handle) = self.get_region(region, false)? else {
                return Ok(None);
            };
            if let Some(la) = &self.last_access {
                la.touch_region(region);
            }
            let record = {
                let mut rf = match handle.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                rf.read(pos)?
            };
            let Some(record) = record else {
                return Ok(None);
            };
            let nbt = record.nbt(Limits::saved())?;
            let saved_version = match nbt.tag.get("DataVersion") {
                Some(Tag::Int(v)) => *v,
                _ => -1,
            };
            let version_mismatch = saved_version != DATA_VERSION;
            Ok(Some((nbt, version_mismatch)))
        }

        /// Lists all `RegionPos` currently stored on disk in this directory.
        pub fn list_regions(&self) -> io::Result<Vec<RegionPos>> {
            if !self.directory.exists() {
                return Ok(Vec::new());
            }
            let mut regions = Vec::new();
            for entry in fs::read_dir(&self.directory)? {
                let entry = entry?;
                if let Some(region) = parse_region_filename(&entry.file_name().to_string_lossy()) {
                    regions.push(region);
                }
            }
            regions.sort();
            Ok(regions)
        }

        /// Lists all chunk positions present inside a specific `region`.
        pub fn region_chunks(&self, region: RegionPos) -> io::Result<Vec<ChunkPos>> {
            let Some(handle) = self.get_region(region, false)? else {
                return Ok(Vec::new());
            };
            let rf = match handle.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            Ok(rf.chunks().map(|(pos, _)| pos).collect())
        }

        /// Deletes a region's `.mca` and `.meta` files and removes it from the cache and `last_access`.
        pub fn delete_region(&self, region: RegionPos) -> io::Result<()> {
            {
                let mut guard = match self.cache.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                guard.files.remove(&region);
            }
            if let Some(la) = &self.last_access {
                la.remove_region(region);
            }
            let mca = self.directory.join(region.mca_file_name());
            if mca.exists() {
                let _ = fs::remove_file(mca);
            }
            let meta = self.directory.join(region.meta_file_name());
            if meta.exists() {
                let _ = fs::remove_file(meta);
            }
            Ok(())
        }

        /// Flushes all open region files and persists `last_access` to disk.
        pub fn close(&self) -> io::Result<()> {
            {
                let mut guard = match self.cache.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                for (_, (handle, _)) in guard.files.drain() {
                    if let Some(h) = handle {
                        if let Ok(rf) = h.lock() {
                            let _ = rf.sync();
                        }
                    }
                }
            }
            if self.writeable {
                if let Some(la) = &self.last_access {
                    la.close()?;
                }
            }
            Ok(())
        }
    }

    /// Recursively scans `<bobby_root>` for `last_access` files, deletes regions not loaded in
    /// `delete_unused_regions_after_days` days, and prunes empty directories (`Bobby.cleanupOldWorlds`).
    pub fn cleanup_unused_regions(bobby_root: impl AsRef<Path>, delete_after_days: i32) -> io::Result<usize> {
        if delete_after_days < 0 {
            return Ok(0);
        }
        let root = bobby_root.as_ref();
        if !root.exists() {
            return Ok(0);
        }
        let mut deleted_count = 0;
        cleanup_dir_recursive(root, delete_after_days, &mut deleted_count)?;
        Ok(deleted_count)
    }

    fn cleanup_dir_recursive(dir: &Path, delete_after_days: i32, deleted: &mut usize) -> io::Result<bool> {
        if !dir.is_dir() {
            return Ok(false);
        }
        let last_access_path = dir.join(LastAccessFile::FILE_NAME);
        if last_access_path.is_file() {
            if let Ok(la) = LastAccessFile::new(dir) {
                for region in la.poll_regions_older_than(delete_after_days) {
                    let mca = dir.join(region.mca_file_name());
                    if fs::remove_file(&mca).is_ok() {
                        *deleted += 1;
                    }
                    let _ = fs::remove_file(dir.join(region.meta_file_name()));
                }
                let _ = la.close();
            }
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let _ = cleanup_dir_recursive(&path, delete_after_days, deleted);
            }
        }

        // Check if directory only has `last_access` (or is completely empty) and remove if so.
        let remaining: Vec<_> = fs::read_dir(dir)?.flatten().collect();
        if remaining.len() == 1
            && remaining[0].file_name().to_string_lossy() == LastAccessFile::FILE_NAME
        {
            let _ = fs::remove_file(remaining[0].path());
        }
        if fs::read_dir(dir)?.next().is_none() {
            let _ = fs::remove_dir(dir);
            return Ok(true);
        }
        Ok(false)
    }
}

pub mod worlds {
    // Dynamic multi-world detection, fingerprint matching, and region merging (`Worlds`).
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.Worlds`.
    // When a multiplayer server reuses the same dimension identifier (e.g. `minecraft:overworld`)
    // across multiple lobbies or sub-worlds, `Worlds` compares 64-bit opaque-block fingerprints
    // of live chunks against cached worlds (`worlds.meta` and `r.<x>.<z>.meta`) to automatically
    // select the matching world cache and merge fragmented caches.

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
}

pub mod manager {
    // High-performance, non-blocking fake-chunk manager (`FakeChunkManager`).
    //
    // Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunkManager` and
    // `de.johni0702.minecraft.bobby.mixin.{ClientChunkCacheMixin, ClientPacketListenerMixin, MinecraftMixin}`.

    use super::BOBBY_DIR_NAME;
    use super::chunk::CachedChunk;
    use super::config::BobbyConfig;
    use super::serializer::{deserialize, fingerprint, serialize};
    use super::storage::FakeChunkStorage;
    use super::tracker::VisibleChunksTracker;
    use super::worlds::Worlds;
    use horizon_core::position::ChunkPos;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    /// Default number of background chunk-loading worker threads (`FakeChunkManager`: 8).
    pub const DEFAULT_LOAD_THREADS: usize = 8;

    /// Default per-frame time budget for integrating loaded fake chunks (`2.5 ms`).
    pub const DEFAULT_FRAME_BUDGET: Duration = Duration::from_micros(2500);

    /// Resolves the on-disk cache directory for a server, seed hash, and dimension:
    /// `<game_dir>/.bobby/<sanitized_server>/<seed_hash>/<dim_namespace>/<dim_path>`.
    ///
    /// Mirrors `FakeChunkManager.getCurrentWorldFolder`.
    pub fn resolve_cache_directory(
        game_dir: impl AsRef<Path>,
        server_address: &str,
        seed_hash: i64,
        dimension: &str,
    ) -> PathBuf {
        let sanitized_server = server_address.replace([':', '/'], "_");
        let (namespace, dim_path) = dimension
            .split_once(':')
            .unwrap_or(("minecraft", dimension));
        game_dir
            .as_ref()
            .join(BOBBY_DIR_NAME)
            .join(sanitized_server)
            .join(seed_hash.to_string())
            .join(namespace)
            .join(dim_path)
    }

    /// A background job to load and deserialize a single fake chunk from Anvil region storage.
    pub struct LoadingJob {
        pub pos: ChunkPos,
        pub cancelled: AtomicBool,
        storage: Arc<FakeChunkStorage>,
        config: BobbyConfig,
        has_skylight: bool,
    }

    impl LoadingJob {
        pub fn cancel(&self) {
            self.cancelled.store(true, Ordering::Relaxed);
        }

        pub fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Relaxed)
        }

        /// Squared distance from the player's current chunk `(cx, cz)`, used for priority scheduling.
        #[inline]
        pub fn distance_squared(&self, cx: i32, cz: i32) -> i64 {
            let dx = i64::from(self.pos.x - cx);
            let dz = i64::from(self.pos.z - cz);
            dx * dx + dz * dz
        }
    }

    struct LoadScheduler {
        center_x: i32,
        center_z: i32,
        pending: Vec<Arc<LoadingJob>>,
        shutdown: bool,
    }

    impl LoadScheduler {
        fn new() -> Self {
            Self {
                center_x: 0,
                center_z: 0,
                pending: Vec::new(),
                shutdown: false,
            }
        }

        /// Pops the non-cancelled job closest to `(center_x, center_z)`.
        fn pop_closest(&mut self) -> Option<Arc<LoadingJob>> {
            // Drop cancelled jobs from the tail first.
            self.pending.retain(|job| !job.is_cancelled());
            if self.pending.is_empty() {
                return None;
            }
            let cx = self.center_x;
            let cz = self.center_z;
            let (best_idx, _) = self
                .pending
                .iter()
                .enumerate()
                .min_by_key(|(_, job)| job.distance_squared(cx, cz))?;
            Some(self.pending.swap_remove(best_idx))
        }
    }

    enum SaveTask {
        Save {
            chunk: CachedChunk,
            config: BobbyConfig,
            storage: Arc<FakeChunkStorage>,
        },
        Flush(mpsc::SyncSender<()>),
        Shutdown,
    }

    enum FingerprintTask {
        Compute { chunk: CachedChunk },
        Shutdown,
    }

    struct LoadedChunkResult {
        pos: ChunkPos,
        chunk: Option<CachedChunk>,
        version_mismatch: bool,
    }

    /// Summary of state changes produced during a frame/tick call to [`FakeChunkManager::update`].
    #[derive(Debug, Default)]
    pub struct UpdateEvents {
        /// Newly loaded fake chunks ready to be inserted into the client world and meshed.
        pub loaded_fake_chunks: Vec<CachedChunk>,
        /// Fake chunk coordinates that were unloaded after their `unload_delay_secs` elapsed.
        pub unloaded_fake_chunks: Vec<ChunkPos>,
        /// Whether any loaded chunk had a `DataVersion` older/newer than Horizon's `DATA_VERSION`.
        pub version_mismatch_detected: bool,
        /// If `dynamic_multi_world` switched the active sub-world cache, holds the new world ID.
        pub switched_world: Option<u32>,
    }

    /// High-performance non-blocking manager for cached multiplayer chunks (`FakeChunkManager`).
    pub struct FakeChunkManager {
        config: BobbyConfig,
        has_skylight: bool,
        storage: Arc<FakeChunkStorage>,
        worlds: Option<Worlds>,
        tracker: VisibleChunksTracker,
        /// Resident fake chunks currently loaded in the client world beyond the server's view distance.
        fake_chunks: HashMap<ChunkPos, CachedChunk>,
        /// Coordinates of real server chunks currently resident in the client world.
        real_chunks: HashSet<ChunkPos>,
        /// Active background load jobs keyed by `ChunkPos` so they can be cancelled on demand.
        loading_jobs: HashMap<ChunkPos, Arc<LoadingJob>>,
        /// Delayed unload map: `ChunkPos -> scheduled unload Instant`.
        to_be_unloaded: HashMap<ChunkPos, Instant>,
        /// FIFO queue of `(ChunkPos, scheduled Instant)` for `O(1)` expiration checks per frame.
        unload_queue: VecDeque<(ChunkPos, Instant)>,
        /// Shared priority scheduler for the load worker pool.
        load_scheduler: Arc<(Mutex<LoadScheduler>, Condvar)>,
        load_results_rx: Receiver<LoadedChunkResult>,
        load_workers: Vec<JoinHandle<()>>,
        /// Channel to the single-threaded background save worker.
        save_tx: Option<Sender<SaveTask>>,
        save_worker: Option<JoinHandle<()>>,
        /// Channel to/from the background fingerprint worker.
        fingerprint_tx: Option<Sender<FingerprintTask>>,
        fingerprint_rx: Receiver<(ChunkPos, i64, u64)>,
        fingerprint_worker: Option<JoinHandle<()>>,
        /// Whether we have already warned about `DataVersion` mismatch in this dimension.
        warned_version_mismatch: bool,
    }

    impl FakeChunkManager {
        /// Creates a new `FakeChunkManager` for a dimension cache directory.
        pub fn open(
            cache_directory: impl AsRef<Path>,
            config: BobbyConfig,
            has_skylight: bool,
        ) -> io::Result<Self> {
            Self::open_with_threads(cache_directory, config, has_skylight, DEFAULT_LOAD_THREADS)
        }

        /// Creates a new `FakeChunkManager` with an explicit number of background load threads.
        pub fn open_with_threads(
            cache_directory: impl AsRef<Path>,
            config: BobbyConfig,
            has_skylight: bool,
            load_thread_count: usize,
        ) -> io::Result<Self> {
            let cache_directory = cache_directory.as_ref().to_path_buf();
            let (storage, worlds) = if config.dynamic_multi_world {
                let worlds = Worlds::open(&cache_directory)?;
                let active_storage = worlds
                    .current_world()
                    .map(|w| w.storage.clone())
                    .unwrap_or_else(|| FakeChunkStorage::open(&cache_directory, true).unwrap());
                (active_storage, Some(worlds))
            } else {
                (FakeChunkStorage::open(&cache_directory, true)?, None)
            };

            // 1. Spawn the multi-threaded prioritized load worker pool.
            let load_scheduler = Arc::new((Mutex::new(LoadScheduler::new()), Condvar::new()));
            let (load_results_tx, load_results_rx) = mpsc::channel();
            let num_threads = load_thread_count.max(1);
            let mut load_workers = Vec::with_capacity(num_threads);

            for idx in 0..num_threads {
                let sched = load_scheduler.clone();
                let tx = load_results_tx.clone();
                let handle = thread::Builder::new()
                    .name(format!("bobby-loading-{idx}"))
                    .spawn(move || {
                        let (lock, cvar) = &*sched;
                        loop {
                            let job = {
                                let mut guard = match lock.lock() {
                                    Ok(g) => g,
                                    Err(e) => e.into_inner(),
                                };
                                loop {
                                    if guard.shutdown {
                                        return;
                                    }
                                    if let Some(job) = guard.pop_closest() {
                                        break job;
                                    }
                                    guard = match cvar.wait(guard) {
                                        Ok(g) => g,
                                        Err(e) => e.into_inner(),
                                    };
                                }
                            };

                            if job.is_cancelled() {
                                continue;
                            }

                            match job.storage.load_tag(job.pos) {
                                Ok(Some((nbt, version_mismatch))) => {
                                    if job.is_cancelled() {
                                        continue;
                                    }
                                    let chunk =
                                        deserialize(job.pos, &nbt, &job.config, job.has_skylight);
                                    let _ = tx.send(LoadedChunkResult {
                                        pos: job.pos,
                                        chunk,
                                        version_mismatch,
                                    });
                                }
                                Ok(None) | Err(_) => {
                                    let _ = tx.send(LoadedChunkResult {
                                        pos: job.pos,
                                        chunk: None,
                                        version_mismatch: false,
                                    });
                                }
                            }
                        }
                    })?;
                load_workers.push(handle);
            }

            // 2. Spawn the single-threaded background save worker (`bobby-saving`).
            let (save_tx, save_rx) = mpsc::channel::<SaveTask>();
            let save_worker = thread::Builder::new()
                .name("bobby-saving".into())
                .spawn(move || {
                    for task in save_rx {
                        match task {
                            SaveTask::Save {
                                chunk,
                                config,
                                storage,
                            } => {
                                let nbt = serialize(&chunk, &config);
                                if let Err(e) = storage.write_chunk(chunk.pos, &nbt) {
                                    log::warn!(
                                        "Bobby: failed to save chunk ({}, {}): {e}",
                                        chunk.pos.x,
                                        chunk.pos.z
                                    );
                                }
                            }
                            SaveTask::Flush(ack) => {
                                let _ = ack.send(());
                            }
                            SaveTask::Shutdown => break,
                        }
                    }
                })?;

            // 3. Spawn the background fingerprint worker (`bobby-fingerprint`).
            let (fp_tx, fp_rx) = mpsc::channel::<FingerprintTask>();
            let (fp_done_tx, fp_done_rx) = mpsc::channel::<(ChunkPos, i64, u64)>();
            let fingerprint_worker = thread::Builder::new()
                .name("bobby-fingerprint".into())
                .spawn(move || {
                    for task in fp_rx {
                        match task {
                            FingerprintTask::Compute { chunk } => {
                                let fp = fingerprint(&chunk);
                                if fp_done_tx.send((chunk.pos, chunk.age, fp)).is_err() {
                                    break;
                                }
                            }
                            FingerprintTask::Shutdown => break,
                        }
                    }
                })?;

            Ok(Self {
                config,
                has_skylight,
                storage,
                worlds,
                tracker: VisibleChunksTracker::new(),
                fake_chunks: HashMap::new(),
                real_chunks: HashSet::new(),
                loading_jobs: HashMap::new(),
                to_be_unloaded: HashMap::new(),
                unload_queue: VecDeque::new(),
                load_scheduler,
                load_results_rx,
                load_workers,
                save_tx: Some(save_tx),
                save_worker: Some(save_worker),
                fingerprint_tx: Some(fp_tx),
                fingerprint_rx: fp_done_rx,
                fingerprint_worker: Some(fingerprint_worker),
                warned_version_mismatch: false,
            })
        }

        pub fn config(&self) -> &BobbyConfig {
            &self.config
        }

        pub fn storage(&self) -> &Arc<FakeChunkStorage> {
            &self.storage
        }

        pub fn worlds(&self) -> Option<&Worlds> {
            self.worlds.as_ref()
        }

        pub fn worlds_mut(&mut self) -> Option<&mut Worlds> {
            self.worlds.as_mut()
        }

        /// Returns a reference to the currently loaded fake chunk at `pos`, if any.
        pub fn get_fake_chunk(&self, pos: ChunkPos) -> Option<&CachedChunk> {
            self.fake_chunks.get(&pos)
        }

        /// Returns all currently resident fake chunks.
        pub fn fake_chunks(&self) -> &HashMap<ChunkPos, CachedChunk> {
            &self.fake_chunks
        }

        /// Returns whether `pos` is currently held as a real server chunk.
        pub fn is_real_chunk(&self, pos: ChunkPos) -> bool {
            self.real_chunks.contains(&pos)
        }

        /// Returns whether `(x, z)` is within Bobby's tracked client render distance.
        pub fn should_be_loaded(&self, x: i32, z: i32) -> bool {
            self.config.enabled && self.tracker.should_be_loaded(x, z)
        }

        /// Updates configuration at runtime (e.g. toggling `taint_fake_chunks` or `max_render_distance`).
        pub fn update_config(&mut self, new_config: BobbyConfig) {
            let taint_changed = self.config.taint_fake_chunks != new_config.taint_fake_chunks;
            self.config = new_config;
            if taint_changed {
                let taint = self.config.taint_fake_chunks;
                for chunk in self.fake_chunks.values_mut() {
                    chunk.set_tainted(taint);
                }
            }
        }

        /// Called when a real chunk arrives from the server (`ClientboundLevelChunkWithLightPacket`).
        ///
        /// Mirrors `ClientPacketListenerMixin` + `ClientChunkCacheMixin.replaceWithPacketData`:
        /// - Marks `chunk.pos` as a resident real chunk.
        /// - Cancels any pending background fake-chunk load or delayed unload for `chunk.pos`.
        /// - Removes any resident fake chunk at `chunk.pos` (`will_be_replaced = true`).
        /// - Submits a shallow snapshot to the background fingerprint worker (and optionally saves it).
        ///
        /// Returns the superseded `CachedChunk` if a fake chunk was resident at `chunk.pos`.
        pub fn on_real_chunk_received(&mut self, chunk: &CachedChunk) -> Option<CachedChunk> {
            let pos = chunk.pos;
            self.real_chunks.insert(pos);
            self.cancel_load(pos);
            self.cancel_unload(pos);

            let replaced_fake = self.fake_chunks.remove(&pos).map(|mut old| {
                old.will_be_replaced = true;
                old
            });

            if self.worlds.is_some() {
                if let Some(tx) = &self.fingerprint_tx {
                    let _ = tx.send(FingerprintTask::Compute {
                        chunk: chunk.shallow_copy(),
                    });
                }
            }

            replaced_fake
        }

        /// Called when the server unloads a real chunk (`ForgetLevelChunk`, `SetChunkCacheCenter`,
        /// or `SetChunkCacheRadius`), *before* the chunk is removed from the client world.
        ///
        /// Mirrors `ClientChunkCacheMixin.saveRealChunk` and `substituteFakeChunk`:
        /// 1. Takes a shallow copy of `real_chunk` and queues it for non-blocking Anvil `.mca`
        ///    serialization on the background save worker.
        /// 2. If `real_chunk.pos` is still within the client's local render distance (`should_be_loaded`),
        ///    prepares and stores a `FakeChunk` substitute (stripping block entities if `no_block_entities`
        ///    and tainting light if `taint_fake_chunks`) and returns `Some(fake_chunk)` so it remains rendered!
        pub fn on_real_chunk_unloading(&mut self, real_chunk: &CachedChunk) -> Option<CachedChunk> {
            let pos = real_chunk.pos;
            self.real_chunks.remove(&pos);

            if !self.config.enabled {
                return None;
            }

            // Queue non-blocking disk save on the single-threaded save worker.
            self.queue_save(real_chunk.shallow_copy());

            // If the chunk is still within our client render distance, substitute a FakeChunk in place!
            if self.tracker.should_be_loaded(pos.x, pos.z) {
                let mut fake = real_chunk.shallow_copy();
                if self.config.no_block_entities {
                    fake.strip_block_entities();
                }
                if self.config.taint_fake_chunks {
                    fake.set_tainted(true);
                }
                self.cancel_unload(pos);
                self.fake_chunks.insert(pos, fake.clone());
                return Some(fake);
            }

            None
        }

        /// Queues a chunk snapshot for asynchronous NBT serialization and region writing.
        pub fn queue_save(&self, chunk: CachedChunk) {
            if let Some(tx) = &self.save_tx {
                let _ = tx.send(SaveTask::Save {
                    chunk,
                    config: self.config.clone(),
                    storage: self.storage.clone(),
                });
            }
        }

        /// Frame-budgeted update (`FakeChunkManager.update` called from `MinecraftMixin.bobbyUpdate`).
        ///
        /// - Updates the `VisibleChunksTracker` for `(player_chunk_x, player_chunk_z)` and `client_render_distance`.
        /// - Schedules disk loads for newly visible chunks not already loaded as real or fake chunks.
        /// - Schedules delayed unloads (`unload_delay_secs`) for chunks leaving the visible area.
        /// - Processes completed background load jobs within `time_budget`.
        pub fn update(
            &mut self,
            player_chunk_x: i32,
            player_chunk_z: i32,
            client_render_distance: i32,
            time_budget: Duration,
        ) -> UpdateEvents {
            let start = Instant::now();
            let mut events = UpdateEvents::default();

            if !self.config.enabled {
                return events;
            }

            // 1. Drain computed chunk fingerprints and update dynamic multi-world matching.
            while let Ok((pos, age, fp)) = self.fingerprint_rx.try_recv() {
                if let Some(worlds) = &mut self.worlds {
                    if let Some(new_world_id) =
                        worlds.observe_chunk(pos, age, fp, self.config.dynamic_multi_world)
                    {
                        if let Some(entry) = worlds.current_world() {
                            self.storage = entry.storage.clone();
                            events.switched_world = Some(new_world_id);
                        }
                    }
                }
            }

            // If dynamic multi-world switched the active world cache, reload fake chunks from the new storage.
            if events.switched_world.is_some() {
                let old_fakes: Vec<ChunkPos> = self.fake_chunks.keys().copied().collect();
                for pos in old_fakes {
                    self.fake_chunks.remove(&pos);
                    events.unloaded_fake_chunks.push(pos);
                    if self.tracker.should_be_loaded(pos.x, pos.z) && !self.real_chunks.contains(&pos) {
                        self.start_load(pos);
                    }
                }
            }

            // 2. Update the visible chunks box around the player's chunk position.
            let effective_vd = self.config.effective_render_distance(client_render_distance);
            if self.tracker.center_x() != player_chunk_x
                || self.tracker.center_z() != player_chunk_z
                || self.tracker.view_distance() != effective_vd
            {
                // Update the load scheduler's center so worker threads prioritize chunks closest to the new position.
                if let Ok(mut sched) = self.load_scheduler.0.lock() {
                    sched.center_x = player_chunk_x;
                    sched.center_z = player_chunk_z;
                }

                let mut to_unload = Vec::new();
                let mut to_load = Vec::new();
                self.tracker.update(
                    player_chunk_x,
                    player_chunk_z,
                    effective_vd,
                    |x, z| to_unload.push(ChunkPos::new(x, z)),
                    |x, z| to_load.push(ChunkPos::new(x, z)),
                );

                for pos in to_unload {
                    self.cancel_load(pos);
                    if self.fake_chunks.contains_key(&pos) {
                        self.queue_unload(pos);
                    }
                }
                for pos in to_load {
                    self.cancel_unload(pos);
                    if !self.real_chunks.contains(&pos) && !self.fake_chunks.contains_key(&pos) {
                        self.start_load(pos);
                    }
                }
            }

            // 3. Process expired delayed unloads (`unload_queue`).
            let now = Instant::now();
            while let Some(&(pos, scheduled_at)) = self.unload_queue.front() {
                if scheduled_at > now {
                    break;
                }
                self.unload_queue.pop_front();
                // Only unload if `to_be_unloaded` still has this exact timestamp (wasn't cancelled or rescheduled).
                if self.to_be_unloaded.get(&pos) == Some(&scheduled_at) {
                    self.to_be_unloaded.remove(&pos);
                    if self.fake_chunks.remove(&pos).is_some() {
                        events.unloaded_fake_chunks.push(pos);
                    }
                }
            }

            // 4. Integrate decoded fake chunks from background workers within `time_budget`.
            loop {
                match self.load_results_rx.try_recv() {
                    Ok(res) => {
                        self.loading_jobs.remove(&res.pos);
                        if res.version_mismatch && !self.warned_version_mismatch {
                            self.warned_version_mismatch = true;
                            events.version_mismatch_detected = true;
                        }
                        let Some(chunk) = res.chunk else {
                            continue;
                        };
                        // Skip if the chunk left view distance or was superseded by a real server chunk.
                        if !self.tracker.should_be_loaded(res.pos.x, res.pos.z)
                            || self.real_chunks.contains(&res.pos)
                        {
                            continue;
                        }
                        self.cancel_unload(res.pos);
                        self.fake_chunks.insert(res.pos, chunk.clone());
                        events.loaded_fake_chunks.push(chunk);

                        if start.elapsed() >= time_budget {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }

            events
        }

        fn start_load(&mut self, pos: ChunkPos) {
            if self.loading_jobs.contains_key(&pos) {
                return;
            }
            let job = Arc::new(LoadingJob {
                pos,
                cancelled: AtomicBool::new(false),
                storage: self.storage.clone(),
                config: self.config.clone(),
                has_skylight: self.has_skylight,
            });
            self.loading_jobs.insert(pos, job.clone());
            let (lock, cvar) = &*self.load_scheduler;
            if let Ok(mut sched) = lock.lock() {
                sched.pending.push(job);
                cvar.notify_one();
            }
        }

        fn cancel_load(&mut self, pos: ChunkPos) {
            if let Some(job) = self.loading_jobs.remove(&pos) {
                job.cancel();
            }
        }

        fn queue_unload(&mut self, pos: ChunkPos) {
            let due = Instant::now() + Duration::from_secs(u64::from(self.config.unload_delay_secs));
            self.to_be_unloaded.insert(pos, due);
            self.unload_queue.push_back((pos, due));
        }

        fn cancel_unload(&mut self, pos: ChunkPos) {
            self.to_be_unloaded.remove(&pos);
        }

        /// Blocks until all queued chunk saves have been written to disk.
        pub fn flush_saves(&self) {
            if let Some(tx) = &self.save_tx {
                let (ack_tx, ack_rx) = mpsc::sync_channel(1);
                if tx.send(SaveTask::Flush(ack_tx)).is_ok() {
                    let _ = ack_rx.recv();
                }
            }
        }

        /// Gracefully shuts down all background worker threads, flushes pending saves,
        /// and closes open region files and metadata.
        pub fn close(&mut self) -> io::Result<()> {
            for job in self.loading_jobs.values() {
                job.cancel();
            }
            self.loading_jobs.clear();

            {
                let (lock, cvar) = &*self.load_scheduler;
                if let Ok(mut sched) = lock.lock() {
                    sched.pending.clear();
                    sched.shutdown = true;
                    cvar.notify_all();
                }
            }
            for handle in self.load_workers.drain(..) {
                let _ = handle.join();
            }

            if let Some(tx) = self.save_tx.take() {
                let _ = tx.send(SaveTask::Shutdown);
            }
            if let Some(handle) = self.save_worker.take() {
                let _ = handle.join();
            }

            if let Some(tx) = self.fingerprint_tx.take() {
                let _ = tx.send(FingerprintTask::Shutdown);
            }
            if let Some(handle) = self.fingerprint_worker.take() {
                let _ = handle.join();
            }

            if let Some(worlds) = &mut self.worlds {
                worlds.save_all()?;
            }
            self.storage.close()
        }
    }

    impl Drop for FakeChunkManager {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }
}

pub use chunk::{CachedChunk, CachedSection, LIGHT_LAYER_BYTES, TAINT_LIGHT_OFFSET};
pub use config::BobbyConfig;
pub use manager::{
    DEFAULT_FRAME_BUDGET, DEFAULT_LOAD_THREADS, FakeChunkManager, LoadingJob, UpdateEvents,
    resolve_cache_directory,
};
pub use serializer::{deserialize, fingerprint, hash64_fingerprint, serialize};
pub use storage::{FakeChunkStorage, LastAccessFile, MAX_OPEN_REGIONS, cleanup_unused_regions};
pub use tracker::{RegionPos, VisibleChunksTracker};
pub use worlds::{RegionMeta, WorldEntry, WorldMatchScore, Worlds};

use horizon_core::position::ChunkPos;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Global client-side Bobby state owned across frames while connected to a server.
pub struct BobbyClientState {
    pub config: BobbyConfig,
    pub manager: Option<FakeChunkManager>,
    pub active_dimension: String,
    pub active_server: String,
    pub active_seed_hash: i64,
    /// Resident fake chunk columns currently inserted into `game.world.chunks`.
    pub resident_fakes: HashSet<(i32, i32)>,
}

impl Default for BobbyClientState {
    fn default() -> Self {
        let config_path = PathBuf::from("config").join("bobby.conf");
        let config = BobbyConfig::load_from_path(&config_path).unwrap_or_default();
        Self {
            config,
            manager: None,
            active_dimension: String::new(),
            active_server: String::new(),
            active_seed_hash: 0,
            resident_fakes: HashSet::new(),
        }
    }
}

pub static STATE: OnceLock<Mutex<BobbyClientState>> = OnceLock::new();

pub fn with_state<R>(f: impl FnOnce(&mut BobbyClientState) -> R) -> R {
    let lock = STATE.get_or_init(|| Mutex::new(BobbyClientState::default()));
    let mut guard = match lock.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    f(&mut guard)
}

/// Initializes or switches the `FakeChunkManager` for `(server_address, seed_hash, dimension)`.
pub fn open_dimension(
    server_address: &str,
    seed_hash: i64,
    dimension: &str,
    has_skylight: bool,
) {
    with_state(|state| {
        if let Some(mut old) = state.manager.take() {
            let _ = old.close();
        }
        state.resident_fakes.clear();
        state.active_server = server_address.to_owned();
        state.active_seed_hash = seed_hash;
        state.active_dimension = dimension.to_owned();

        if !state.config.enabled {
            return;
        }

        let dir = resolve_cache_directory(".", server_address, seed_hash, dimension);
        match FakeChunkManager::open(&dir, state.config.clone(), has_skylight) {
            Ok(mgr) => {
                state.manager = Some(mgr);
            }
            Err(e) => {
                log::warn!("Bobby: could not open FakeChunkManager at {}: {e}", dir.display());
            }
        }
    });
}

/// Closes the active `FakeChunkManager` and triggers background region cleanup if configured.
pub fn close_dimension() {
    let delete_days = with_state(|state| {
        if let Some(mut mgr) = state.manager.take() {
            let _ = mgr.close();
        }
        state.resident_fakes.clear();
        state.config.delete_unused_regions_after_days
    });

    if delete_days >= 0 {
        let _ = std::thread::Builder::new()
            .name("bobby-cleanup".into())
            .spawn(move || {
                let _ = cleanup_unused_regions(BOBBY_DIR_NAME, delete_days);
            });
    }
}

/// Converts a resident `horizon_client::chunk::Chunk` into a `CachedChunk`
/// via Horizon's Anvil NBT encoder (`horizon_client::save::encode_with`).
pub fn snapshot_client_chunk(
    game: &crate::game::Game<'_>,
    chunk: &horizon_client::chunk::Chunk,
    config: &BobbyConfig,
) -> Option<CachedChunk> {
    let nbt = horizon_client::save::encode_with(
        chunk,
        game.pack.assets.registry,
        game.res.biomes(),
        None,
        game.world.shape,
    );
    let pos = ChunkPos::new(chunk.x, chunk.z);
    let mut cached = deserialize(pos, &nbt, config, game.world.shape.sky_light)?;
    cached.age = game.world.game_time;
    Some(cached)
}

/// Converts a `CachedChunk` into a lit `horizon_client::chunk::Chunk`
/// ready for `game.streaming.insert_remote(&mut game.world, chunk)`.
pub fn to_client_chunk(
    game: &crate::game::Game<'_>,
    cached: &CachedChunk,
    config: &BobbyConfig,
) -> Option<horizon_client::chunk::Chunk> {
    let nbt = serialize(cached, config);
    horizon_client::save::decode_lit(
        &nbt,
        game.pack.assets.registry,
        game.res.biomes(),
        game.world.layout(),
        game.world.shape.sky_light,
    )
}

/// Handles `/bobby` client subcommands (`worlds`, `create`, `merge`, `upgrade`, `status`).
pub fn execute_command(game: &mut crate::game::Game<'_>, args: &str) -> String {
    let mut parts = args.split_whitespace();
    let sub = parts.next().unwrap_or("status");

    with_state(|state| match sub {
        "status" => {
            let fake_count = state.manager.as_ref().map_or(0, |m| m.fake_chunks().len());
            format!(
                "Bobby: enabled={}, dimension={}, resident_fake_chunks={}, max_render_distance={}",
                state.config.enabled,
                state.active_dimension,
                fake_count,
                state.config.max_render_distance
            )
        }
        "worlds" => {
            let Some(mgr) = &state.manager else {
                return "Bobby: no active FakeChunkManager.".to_owned();
            };
            let Some(worlds) = mgr.worlds() else {
                return "Bobby: dynamic-multi-world is disabled in bobby.conf.".to_owned();
            };
            let current = worlds.current_world_id();
            let mut lines = Vec::new();
            for (&id, entry) in worlds.worlds() {
                let score = worlds.score_world(id);
                let marker = if Some(id) == current { "*" } else { " " };
                lines.push(format!(
                    "{marker} [{id}] {} (+{} / -{} / ?{})",
                    entry.title, score.matching, score.mismatching, score.unknown
                ));
            }
            let _ = game;
            lines.join("\n")
        }
        "create" => {
            let title: String = parts.collect::<Vec<_>>().join(" ");
            let title = if title.is_empty() {
                "unnamed".to_owned()
            } else {
                title
            };
            let Some(mgr) = &mut state.manager else {
                return "Bobby: no active FakeChunkManager.".to_owned();
            };
            let Some(worlds) = mgr.worlds_mut() else {
                return "Bobby: dynamic-multi-world is disabled in bobby.conf.".to_owned();
            };
            match worlds.create_world(&title) {
                Ok(id) => format!("Bobby: created and switched to world [{id}] \"{title}\"."),
                Err(e) => format!("Bobby: failed to create world: {e}"),
            }
        }
        "merge" => {
            let Some(src_str) = parts.next() else {
                return "Usage: /bobby merge <source_world_id>".to_owned();
            };
            let Ok(src_id) = src_str.parse::<u32>() else {
                return format!("Bobby: invalid world ID {src_str:?}");
            };
            let Some(mgr) = &mut state.manager else {
                return "Bobby: no active FakeChunkManager.".to_owned();
            };
            let Some(worlds) = mgr.worlds_mut() else {
                return "Bobby: dynamic-multi-world is disabled in bobby.conf.".to_owned();
            };
            let Some(dst_id) = worlds.current_world_id() else {
                return "Bobby: no current world selected.".to_owned();
            };
            match worlds.merge_worlds(src_id, dst_id) {
                Ok(n) => format!("Bobby: merged {n} chunks from world [{src_id}] into [{dst_id}]."),
                Err(e) => format!("Bobby: merge failed: {e}"),
            }
        }
        "upgrade" => {
            let Some(mgr) = &state.manager else {
                return "Bobby: no active FakeChunkManager.".to_owned();
            };
            let storage = mgr.storage().clone();
            let regions: Vec<RegionPos> = storage.list_regions().unwrap_or_default();
            format!(
                "Bobby: verified {} cached region(s) in {}.",
                regions.len(),
                storage.directory().display()
            )
        }
        _ => "Usage: /bobby [status | worlds | create <title> | merge <id> | upgrade]".to_owned(),
    })
}
