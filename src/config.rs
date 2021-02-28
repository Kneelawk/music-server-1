use crate::error::{ErrorKind::ConfigLoadError, Result, ResultExt};
use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "music-server-1.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ConfigRaw {
    #[serde(default)]
    general: ConfigGeneral,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ConfigGeneral {
    #[serde(rename = "base-dir", default = "default_base_dir")]
    base_dir: String,
    #[serde(
        rename = "media-include-patterns",
        default = "default_media_include_patterns"
    )]
    media_include_patterns: Vec<String>,
    #[serde(rename = "media-exclude-patterns", default)]
    media_exclude_patterns: Vec<String>,
    #[serde(
        rename = "cover-include-patterns",
        default = "default_cover_include_patterns"
    )]
    cover_include_patterns: Vec<String>,
    #[serde(rename = "cover-exclude-patterns", default)]
    cover_exclude_patterns: Vec<String>,
    #[serde(default = "default_bindings")]
    bindings: Vec<String>,
}

impl Default for ConfigGeneral {
    fn default() -> Self {
        ConfigGeneral {
            base_dir: default_base_dir(),
            media_include_patterns: default_media_include_patterns(),
            media_exclude_patterns: Default::default(),
            cover_include_patterns: default_cover_include_patterns(),
            cover_exclude_patterns: Default::default(),
            bindings: default_bindings(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub base_dir: PathBuf,
    pub media_include_patterns: RegexSet,
    pub media_exclude_patterns: RegexSet,
    pub cover_include_patterns: RegexSet,
    pub cover_exclude_patterns: RegexSet,
    pub bindings: Vec<String>,
}

impl Config {
    pub fn load() -> Result<Config> {
        info!("Loading config: {}", CONFIG_FILE_NAME);

        let cfg_path = Path::new(CONFIG_FILE_NAME);

        let cfg_raw: ConfigRaw = if cfg_path.exists() {
            let mut cfg_file = File::open(cfg_path)
                .chain_err(|| ConfigLoadError("Error opening config file".into()))?;
            let mut cfg_string = String::new();
            cfg_file
                .read_to_string(&mut cfg_string)
                .chain_err(|| ConfigLoadError("Error reading config file".into()))?;
            toml::from_str(&cfg_string)
                .chain_err(|| ConfigLoadError("Error decoding config".into()))?
        } else {
            info!("Loading blank cfg file...");
            toml::from_str("").chain_err(|| ConfigLoadError("Error loading blank config".into()))?
        };

        debug!("Writing config file...");
        let new_cfg_string = toml::to_string_pretty(&cfg_raw)
            .chain_err(|| ConfigLoadError("Error re-encoding config file".into()))?;
        let mut cfg_file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(cfg_path)
            .chain_err(|| ConfigLoadError("Error opening config file for re-encoding".into()))?;
        cfg_file
            .write_all(new_cfg_string.as_bytes())
            .chain_err(|| ConfigLoadError("Error writing to config file for re-encoding".into()))?;

        Ok(Config {
            base_dir: cfg_raw.general.base_dir.into(),
            media_include_patterns: RegexSet::new(cfg_raw.general.media_include_patterns)
                .chain_err(|| ConfigLoadError("Error decoding regex".into()))?,
            media_exclude_patterns: RegexSet::new(cfg_raw.general.media_exclude_patterns)
                .chain_err(|| ConfigLoadError("Error decoding regex".into()))?,
            cover_include_patterns: RegexSet::new(cfg_raw.general.cover_include_patterns)
                .chain_err(|| ConfigLoadError("Error decoding regex".into()))?,
            cover_exclude_patterns: RegexSet::new(cfg_raw.general.cover_exclude_patterns)
                .chain_err(|| ConfigLoadError("Error decoding regex".into()))?,
            bindings: cfg_raw.general.bindings,
        })
    }
}

fn default_base_dir() -> String {
    match dirs::audio_dir() {
        None => match dirs::home_dir() {
            None => "~/Music/".to_string(),
            Some(home) => {
                let mut dir = home.clone();
                dir.push("Music");
                dir.to_string_lossy().to_string()
            }
        },
        Some(dir) => dir.to_string_lossy().to_string(),
    }
}

fn default_media_include_patterns() -> Vec<String> {
    vec![
        ".*\\.flac$".to_string(),
        ".*\\.mp3$".to_string(),
        ".*\\.ogg$".to_string(),
    ]
}

fn default_cover_include_patterns() -> Vec<String> {
    vec![".*\\.jpg$".to_string(), ".*\\.png$".to_string()]
}

fn default_bindings() -> Vec<String> {
    vec!["127.0.0.1:8980".to_string()]
}
