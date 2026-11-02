//! High-performance, non-blocking fake-chunk manager (`FakeChunkManager`).
//!
//! Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunkManager` and
//! `de.johni0702.minecraft.bobby.mixin.{ClientChunkCacheMixin, ClientPacketListenerMixin, MinecraftMixin}`.

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicUsize;

    static NEXT_MGR_DIR: AtomicUsize = AtomicUsize::new(0);

    fn temp_mgr_dir() -> PathBuf {
        let id = NEXT_MGR_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "horizon-bobby-mgr-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn manager_saves_unloaded_real_chunks_and_reloads_beyond_server_view_distance() {
        let dir = temp_mgr_dir();
        let mut config = BobbyConfig::default();
        config.unload_delay_secs = 0; // Immediate unload when leaving view distance for test speed
        config.max_render_distance = 16;

        let mut mgr = FakeChunkManager::open_with_threads(&dir, config, true, 2).unwrap();

        // Player is at (0, 0) with render distance 4.
        let _ = mgr.update(0, 0, 4, Duration::from_millis(50));

        // Server sends real chunk at (2, 1).
        let pos = ChunkPos::new(2, 1);
        let real_chunk = CachedChunk::new(pos, -4, 24, true, 777);
        assert!(mgr.on_real_chunk_received(&real_chunk).is_none());
        assert!(mgr.is_real_chunk(pos));

        // Server unloads (2, 1) while player is still at (0, 0) (so (2, 1) is inside render distance 4):
        // Bobby immediately substitutes a FakeChunk AND saves it to disk in the background!
        let substituted = mgr
            .on_real_chunk_unloading(&real_chunk)
            .expect("should substitute fake chunk inside client render distance");
        assert_eq!(substituted.pos, pos);
        assert!(mgr.get_fake_chunk(pos).is_some());
        mgr.flush_saves();

        // Player teleports far away to (100, 100): (2, 1) leaves client render distance and unloads.
        let ev_leave = mgr.update(100, 100, 4, Duration::from_millis(50));
        assert!(ev_leave.unloaded_fake_chunks.contains(&pos));
        assert!(mgr.get_fake_chunk(pos).is_none());

        // Player returns to (0, 0): Bobby's background load pool loads (2, 1) from the Anvil region cache!
        let _ = mgr.update(0, 0, 4, Duration::from_millis(50));
        let mut loaded = false;
        for _ in 0..50 {
            let ev = mgr.update(0, 0, 4, Duration::from_millis(50));
            if ev.loaded_fake_chunks.iter().any(|c| c.pos == pos) {
                loaded = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(loaded, "expected (2, 1) to be loaded asynchronously from disk");
        assert_eq!(mgr.get_fake_chunk(pos).unwrap().age, 777);

        mgr.close().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}

