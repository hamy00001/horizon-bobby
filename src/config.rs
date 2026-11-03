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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_hocon_configs() {
        let hocon = r#"
            # Bobby configuration file
            enabled = true
            dynamic-multi-world = true
            no-block-entities = false
            taint-fake-chunks = true
            unload-delay-secs = 120
            delete-unused-regions-after-days = 14
            max-render-distance = 64
            view-distance-overwrite = 12
        "#;
        let parsed = BobbyConfig::parse_str(hocon).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.dynamic_multi_world);
        assert!(!parsed.no_block_entities);
        assert!(parsed.taint_fake_chunks);
        assert_eq!(parsed.unload_delay_secs, 120);
        assert_eq!(parsed.delete_unused_regions_after_days, 14);
        assert_eq!(parsed.max_render_distance, 64);
        assert_eq!(parsed.view_distance_overwrite, 12);

        let json = serde_json::to_string_pretty(&parsed).unwrap();
        let round_trip = BobbyConfig::parse_str(&json).unwrap();
        assert_eq!(parsed, round_trip);
    }
}

