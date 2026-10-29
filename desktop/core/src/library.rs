//! The library folder: `_library/library.json` (its id, name and settings) and one
//! folder per project at `<host>/<owner>/<name>/`, holding `repo.json` (what the
//! app knows about it), `source/<date>_<commit>/` (each captured version of the
//! code) and `releases/<tag>/` (each captured release's files). Everything the
//! app shows comes from these files, so a library can be copied, synced or
//! opened on another computer (docs/PLAN.md, "The library folder").

use crate::sources::{Account, Details, SourceRef};
use crate::util::{now_iso, read_json_object, write_json};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FORMAT: u64 = 1;
pub const META_DIR: &str = "_library";
pub const REPO_FILE: &str = "repo.json";
/// Downloads in progress, inside a project's folder (removed before each update).
pub const PARTIAL_DIR: &str = ".partial";

/// What to keep of a release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Files {
    /// Every file in the release.
    #[default]
    All,
    /// Files for this computer's system, plus files for any system (checksums, data).
    ThisComputer,
    /// Only the notes, no files.
    None,
}

/// What a project follows, and how often it's checked.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Follow {
    /// Capture the code of `branch` (the default branch when empty).
    pub source: bool,
    pub branch: String,
    /// Capture the latest release.
    pub releases: bool,
    pub prereleases: bool,
    pub files: Files,
    /// Hours between checks: None follows the library's setting, 0 is never (only by hand).
    pub every_hours: Option<f64>,
}

impl Default for Follow {
    fn default() -> Self {
        Self { source: true, branch: String::new(), releases: true, prereleases: false, files: Files::All, every_hours: None }
    }
}

/// One captured version of the code.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// Under `source/`.
    pub folder: String,
    pub branch: String,
    pub commit: String,
    pub commit_date: String,
    pub message: String,
    pub captured: String,
    pub bytes: u64,
    pub files: u64,
    /// The readme, relative to the snapshot's folder ("" when there's none).
    pub readme: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetRecord {
    pub name: String,
    /// The file, relative to the release's folder ("" when it wasn't downloaded).
    pub file: String,
    pub bytes: u64,
    /// Why it wasn't downloaded ("for another system", "over the size limit").
    pub skipped: String,
}

/// One captured release.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReleaseRecord {
    /// Under `releases/`.
    pub folder: String,
    pub tag: String,
    pub name: String,
    pub published: String,
    pub prerelease: bool,
    pub captured: String,
    pub web_url: String,
    pub assets: Vec<AssetRecord>,
    pub bytes: u64,
}

/// What Launch starts: a file in a release's folder ("" for the best one found).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchPick {
    pub file: String,
}

/// `repo.json`. Keys the app doesn't know are kept when it's rewritten.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repo {
    pub source: SourceRef,
    #[serde(default)]
    pub added: String,
    #[serde(default)]
    pub details: Details,
    #[serde(default)]
    pub follow: Follow,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub last_checked: String,
    /// When a new version was last captured.
    #[serde(default)]
    pub last_changed: String,
    #[serde(default)]
    pub last_error: String,
    /// Oldest first.
    #[serde(default)]
    pub snapshots: Vec<Snapshot>,
    /// Oldest first.
    #[serde(default)]
    pub releases: Vec<ReleaseRecord>,
    #[serde(default)]
    pub launch: LaunchPick,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Repo {
    pub fn new(source: SourceRef, follow: Follow) -> Self {
        Self {
            source,
            added: now_iso(),
            details: Details::default(),
            follow,
            tags: vec![],
            notes: String::new(),
            last_checked: String::new(),
            last_changed: String::new(),
            last_error: String::new(),
            snapshots: vec![],
            releases: vec![],
            launch: LaunchPick::default(),
            extra: Map::new(),
        }
    }
    pub fn id(&self) -> String {
        self.source.id()
    }
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.snapshots.last()
    }
    pub fn latest_release(&self) -> Option<&ReleaseRecord> {
        self.releases.last()
    }
    pub fn bytes(&self) -> u64 {
        self.snapshots.iter().map(|s| s.bytes).sum::<u64>() + self.releases.iter().map(|r| r.bytes).sum::<u64>()
    }
}

/// The library's own settings, in library.json ("settings"), so they travel with it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Hours between checks for every project that follows the library (0: only by hand).
    pub check_every_hours: f64,
    /// What a newly added project follows.
    pub new_follow: Follow,
    /// Release files over this many MB aren't downloaded (0: no limit).
    pub max_file_mb: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { check_every_hours: 24.0, new_follow: Follow::default(), max_file_mb: 0 }
    }
}

/// An account whose new projects are added at each check (_library/accounts.json).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FollowedAccount {
    #[serde(flatten)]
    pub account: Account,
    /// What its projects follow when they're added.
    pub follow: Follow,
    /// Whether its forks of other projects are added too.
    #[serde(default)]
    pub forks: bool,
    #[serde(default)]
    pub added: String,
    #[serde(default)]
    pub last_checked: String,
    /// Its projects removed from the library by hand, which aren't added again.
    #[serde(default)]
    pub removed: Vec<String>,
}

pub struct Library {
    pub root: PathBuf,
    pub id: String,
    pub name: String,
    pub settings: Settings,
    /// Every project, by id ("github.com/owner/name").
    pub repos: BTreeMap<String, Repo>,
    /// Folders with a repo.json that couldn't be read: (folder, why).
    pub unreadable: Vec<(String, String)>,
}

impl Library {
    /// Open the library at `root`, making it (and library.json) when it's new.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).with_context(|| format!("couldn't make the library folder {}", root.display()))?;
        let file = root.join(META_DIR).join("library.json");
        let mut v = read_json_object(&file);
        let fresh = v.get("format").is_none();
        if let Some(f) = v["format"].as_u64() {
            if f > FORMAT {
                bail!("This library was made by a newer Repo Sync (format {f}); update the app to open it.");
            }
        }
        if fresh {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Repo Sync".into());
            v = json!({ "format": FORMAT, "id": new_id(), "name": name, "created": now_iso(), "settings": Settings::default() });
            write_json(&file, &v)?;
        }
        let settings: Settings = serde_json::from_value(v["settings"].clone()).unwrap_or_default();
        let mut lib = Self {
            root: root.to_path_buf(),
            id: v["id"].as_str().unwrap_or("").to_string(),
            name: v["name"].as_str().unwrap_or("").to_string(),
            settings,
            repos: BTreeMap::new(),
            unreadable: vec![],
        };
        lib.scan();
        Ok(lib)
    }

    /// Read every project's repo.json again.
    pub fn scan(&mut self) {
        self.repos.clear();
        self.unreadable.clear();
        let mut stack: Vec<(PathBuf, usize)> = vec![(self.root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            let file = dir.join(REPO_FILE);
            if depth > 0 && file.is_file() {
                match std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Repo>(&b)?)) {
                    Ok(r) if self.folder_of(&r.source.id()) == dir => {
                        self.repos.insert(r.id(), r);
                    }
                    Ok(r) => self.unreadable.push((rel(&self.root, &dir), format!("its repo.json is for {}", r.id()))),
                    Err(e) => self.unreadable.push((rel(&self.root, &dir), e.to_string())),
                }
                continue; // a project's own folders hold no other projects
            }
            if depth >= 6 {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.starts_with('_') || !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                stack.push((e.path(), depth + 1));
            }
        }
    }

    pub fn folder_of(&self, id: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in id.split('/') {
            p.push(part);
        }
        p
    }

    fn accounts_file(&self) -> PathBuf {
        self.root.join(META_DIR).join("accounts.json")
    }

    /// The accounts followed for new projects.
    pub fn accounts(&self) -> Vec<FollowedAccount> {
        std::fs::read(self.accounts_file()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| serde_json::from_value(v["accounts"].clone()).ok()).unwrap_or_default()
    }

    pub fn save_accounts(&self, list: &[FollowedAccount]) -> Result<()> {
        write_json(&self.accounts_file(), &json!({ "accounts": list }))
    }

    pub fn save_repo(&self, r: &Repo) -> Result<()> {
        let v = serde_json::to_value(r)?;
        write_json(&self.folder_of(&r.id()).join(REPO_FILE), &v)
    }

    pub fn save_settings(&mut self, s: Settings) -> Result<()> {
        let file = self.root.join(META_DIR).join("library.json");
        let mut v = read_json_object(&file);
        v["settings"] = serde_json::to_value(&s)?;
        write_json(&file, &v)?;
        self.settings = s;
        Ok(())
    }

    /// A path inside the library from a relative one with "/" (library:// addresses), or an error.
    pub fn inside(&self, rel_path: &str) -> Result<PathBuf> {
        let r = crate::util::safe_rel(Path::new(rel_path)).context("not a path inside the library")?;
        Ok(self.root.join(r))
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Kind;

    #[test]
    fn library_round_trip_keeps_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("reposync-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = Library::open(&dir).unwrap();
        assert!(dir.join("_library/library.json").is_file());
        let src = SourceRef { kind: Kind::GitLab, host: "gitlab.com".into(), path: "g/sub/p".into() };
        let mut r = Repo::new(src, Follow::default());
        r.extra.insert("mine".into(), json!(42));
        lib.save_repo(&r).unwrap();
        assert!(dir.join("gitlab.com/g/sub/p/repo.json").is_file());
        // a stray folder with a broken repo.json is reported, not fatal
        std::fs::create_dir_all(dir.join("github.com/x/y")).unwrap();
        std::fs::write(dir.join("github.com/x/y/repo.json"), "{").unwrap();
        let lib = Library::open(&dir).unwrap();
        assert_eq!(lib.repos.len(), 1);
        assert_eq!(lib.repos["gitlab.com/g/sub/p"].extra["mine"], json!(42));
        assert_eq!(lib.unreadable.len(), 1);
        assert!(lib.inside("../etc").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
