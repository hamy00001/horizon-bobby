pub mod chunk;
pub mod config;
pub mod manager;
pub mod serializer;
pub mod storage;
pub mod tracker;
pub mod worlds;

pub use chunk::{CachedChunk, CachedSection, LIGHT_LAYER_BYTES, TAINT_LIGHT_OFFSET};
pub use config::BobbyConfig;
pub use manager::{
    DEFAULT_FRAME_BUDGET, DEFAULT_LOAD_THREADS, FakeChunkManager, LoadingJob, UpdateEvents,
    resolve_cache_directory,
};
pub use serializer::{deserialize, fingerprint, serialize};
pub use storage::{FakeChunkStorage, LastAccessFile, cleanup_unused_regions};
pub use tracker::{RegionPos, VisibleChunksTracker};
pub use worlds::{RegionMeta, WorldEntry, WorldMatchScore, Worlds};

/// Horizon 26.3 Anvil DataVersion (`horizon_world::DATA_VERSION`).
pub const DATA_VERSION: i32 = horizon_world::DATA_VERSION;

/// Root directory name for cached multiplayer chunks (`<game_dir>/.bobby`).
pub const BOBBY_DIR_NAME: &str = ".bobby";

#[cfg(test)]
mod hnmod_tests {
    use std::path::Path;

    #[test]
    fn packaged_hnmod_passes_horizon_native_bundle_validation() {
        let hnmod_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("bobby.hnmod");
        assert!(hnmod_path.exists(), "bobby.hnmod must exist");
        let bytes = std::fs::read(&hnmod_path).expect("read bobby.hnmod");
        // Verify ZIP local file header magic PK\x03\x04 and required archive entries
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let raw = String::from_utf8_lossy(&bytes);
        assert!(raw.contains("horizon-native.json"));
        assert!(raw.contains("mixins/bobby/horizon-mixin.toml"));
        assert!(raw.contains("mixins/bobby/mixins/bobby_mod.rs"));
        assert!(raw.contains("mixins/bobby/mixins/frame.rs"));
    }
}


