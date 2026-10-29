//! What the app keeps on this computer (the system's per-user config folder), as
//! opposed to the portable library folder: which library is open, libraries
//! opened before, tokens for the servers, and the page's preferences.

use crate::sources::Tokens;
use crate::util::{read_json_object, write_atomic};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `~/Repo Sync`, until the user chooses another folder.
pub fn default_library_path() -> PathBuf {
    home_dir().join("Repo Sync")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub library: Option<PathBuf>,
    #[serde(default)]
    pub recent_libraries: Vec<PathBuf>,
    /// Tokens by host. Kept on this computer only, never in the library.
    #[serde(default)]
    pub tokens: Tokens,
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }

    pub fn library_path(&self) -> PathBuf {
        self.library.clone().unwrap_or_else(default_library_path)
    }

    pub fn set_library(&mut self, dir: &Path) {
        self.library = Some(dir.to_path_buf());
        self.recent_libraries.retain(|p| p != dir);
        self.recent_libraries.insert(0, dir.to_path_buf());
        self.recent_libraries.truncate(8);
    }
}

/// Preferences the page stores (a JSON object: theme, view, sort).
pub struct Prefs {
    path: PathBuf,
}

impl Prefs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn get(&self) -> Value {
        read_json_object(&self.path)
    }
    pub fn set(&self, prefs: &Value) -> Result<()> {
        if !prefs.is_object() {
            bail!("preferences must be an object");
        }
        write_atomic(&self.path, &serde_json::to_vec_pretty(prefs)?)
    }
}
