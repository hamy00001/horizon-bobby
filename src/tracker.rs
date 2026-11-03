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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tracker_matches_naive_box_diff_across_moves_and_resizes() {
        let mut tracker = VisibleChunksTracker::new();
        let mut active: HashSet<(i32, i32)> = HashSet::new();

        let steps = [
            (0, 0, 2),
            (1, 0, 2),
            (1, -2, 3),
            (10, 10, 2),
            (9, 10, 1),
            (0, 0, -1),
        ];

        for (cx, cz, vd) in steps {
            let mut unloaded = Vec::new();
            let mut loaded = Vec::new();
            tracker.update(
                cx,
                cz,
                vd,
                |x, z| unloaded.push((x, z)),
                |x, z| loaded.push((x, z)),
            );
            for (x, z) in unloaded {
                assert!(active.remove(&(x, z)), "unloaded chunk not active: ({x}, {z})");
            }
            for (x, z) in loaded {
                assert!(active.insert((x, z)), "loaded chunk already active: ({x}, {z})");
            }

            let mut expected = HashSet::new();
            for x in (cx - vd)..=(cx + vd) {
                for z in (cz - vd)..=(cz + vd) {
                    expected.insert((x, z));
                    assert!(tracker.should_be_loaded(x, z));
                }
            }
            assert_eq!(active, expected, "mismatch at step ({cx}, {cz}, {vd})");
        }
    }

    #[test]
    fn region_pos_conversions() {
        let r = RegionPos::from_chunk_coords(-1, 32);
        assert_eq!(r, RegionPos::new(-1, 1));
        assert_eq!(r.mca_file_name(), "r.-1.1.mca");
        assert_eq!(r.meta_file_name(), "r.-1.1.meta");
        assert_eq!(RegionPos::from_long(r.to_long()), r);
    }
}

