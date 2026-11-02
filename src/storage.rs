//! Anvil region storage (`r.<x>.<z>.mca`) and `last_access` region timestamp tracking.
//!
//! Direct Rust port of `de.johni0702.minecraft.bobby.FakeChunkStorage` and
//! `de.johni0702.minecraft.bobby.LastAccessFile`, built on top of
//! `horizon_world::region::{RegionFile, ChunkRecord}` and `Compression::Zlib`.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::CachedChunk;
    use crate::config::BobbyConfig;
    use crate::serializer::{deserialize, serialize};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("horizon-bobby-{prefix}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn storage_writes_and_reads_compressed_anvil_chunks() {
        let dir = temp_test_dir("storage");
        let storage = FakeChunkStorage::open(&dir, true).unwrap();
        let cfg = BobbyConfig::default();

        let pos = ChunkPos::new(-12, 34);
        let chunk = CachedChunk::new(pos, -4, 24, true, 999);
        let nbt = serialize(&chunk, &cfg);

        storage.write_chunk(pos, &nbt).unwrap();
        storage.close().unwrap();

        let reopened = FakeChunkStorage::open(&dir, false).unwrap();
        let (loaded_nbt, mismatch) = reopened.load_tag(pos).unwrap().expect("chunk exists");
        assert!(!mismatch);
        let decoded = deserialize(pos, &loaded_nbt, &cfg, true).unwrap();
        assert_eq!(decoded.pos, pos);
        assert_eq!(decoded.age, 999);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_access_roundtrip_and_expiration() {
        let dir = temp_test_dir("last-access");
        fs::create_dir_all(&dir).unwrap();
        let la = LastAccessFile::new(&dir).unwrap();
        let r1 = RegionPos::new(0, 0);
        let r2 = RegionPos::new(-2, 5);
        la.touch_region(r1);
        la.map
            .lock()
            .unwrap()
            .insert(r2, la.today_ms - 10 * MILLIS_PER_DAY);
        la.close().unwrap();

        let la2 = LastAccessFile::new(&dir).unwrap();
        let expired = la2.poll_regions_older_than(5);
        assert_eq!(expired, vec![r2]);
        assert!(la2.poll_regions_older_than(0).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}

