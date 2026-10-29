//! Checking a project and capturing what's new: its details, a new version of
//! its code (when the branch has moved on) and a new release (when there is
//! one). Earlier versions and releases are never touched: each new one gets its
//! own folder beside them (docs/PLAN.md, "Updates").

use crate::archive;
use crate::http::{Client, Progress};
use crate::launch::for_this_computer;
use crate::library::{AssetRecord, Files, ReleaseRecord, Repo, Snapshot, PARTIAL_DIR};
use crate::sources::{Release, Source};
use crate::util::{dir_size, now_iso, now_unix, safe_name, stamp_from_unix, unix_from_iso};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::sync::Mutex;

/// What a check found and kept.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Outcome {
    /// The new version's folder under source/, when there was one.
    pub snapshot: Option<String>,
    /// The new release's tag, when there was one.
    pub release: Option<String>,
    /// Something went wrong with one part but the rest was done.
    pub warnings: Vec<String>,
    /// Older versions and releases deleted to stay within the limits to keep.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// What a job that isn't a check did, in words (an account's projects queued).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Limits from the library's settings.
pub struct Limits {
    /// Release files over this many bytes aren't downloaded (0: no limit).
    pub max_file: u64,
    /// Versions of the code to keep, newest first (0: every one).
    pub keep_source: u32,
    /// Releases to keep, newest first, not counting those picked by hand (0: every one).
    pub keep_releases: u32,
}

/// The step in hand, for the Activity page.
pub type Stage = Mutex<String>;

fn set(stage: &Stage, s: impl Into<String>) {
    *stage.lock().unwrap() = s.into();
}

/// Check `repo` (in `folder`) and capture what's new. `repo` is updated in place;
/// the caller saves it.
pub fn update(repo: &mut Repo, folder: &Path, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage, limits: &Limits) -> Result<Outcome> {
    let partial = folder.join(PARTIAL_DIR);
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).with_context(|| format!("couldn't make {}", partial.display()))?;
    let mut out = Outcome::default();

    set(stage, "Reading the project's details");
    let details = src.details()?;
    repo.details = details;

    if repo.follow.source {
        match capture_source(repo, folder, src, client, progress, stage) {
            Ok(s) => out.snapshot = s,
            Err(e) if progress.stopped() => return Err(e),
            Err(e) => out.warnings.push(format!("Source: {e:#}")),
        }
    }
    if repo.follow.releases {
        match capture_release(repo, folder, src, client, progress, stage, limits) {
            Ok(r) => out.release = r,
            Err(e) if progress.stopped() => return Err(e),
            Err(e) => out.warnings.push(format!("Release: {e:#}")),
        }
    }
    let _ = std::fs::remove_dir_all(&partial);
    out.removed = prune(repo, folder, limits);
    repo.last_checked = now_iso();
    if out.snapshot.is_some() || out.release.is_some() {
        repo.last_changed = repo.last_checked.clone();
    }
    repo.last_error = out.warnings.join("\n");
    Ok(out)
}

fn capture_source(repo: &mut Repo, folder: &Path, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage) -> Result<Option<String>> {
    let branch = if repo.follow.branch.trim().is_empty() { repo.details.default_branch.clone() } else { repo.follow.branch.trim().to_string() };
    if branch.is_empty() {
        bail!("the project has no default branch (is it empty?)");
    }
    set(stage, format!("Checking the {branch} branch"));
    let commit = src.commit(&branch)?;
    if repo.snapshots.iter().any(|s| s.commit == commit.sha) {
        return Ok(None);
    }
    let short = &commit.sha[..commit.sha.len().min(7)];
    set(stage, format!("Downloading the code at {short}"));
    let partial = folder.join(PARTIAL_DIR);
    let zip = partial.join("source.zip");
    let (url, headers) = src.source_zip(&commit.sha);
    client.download(&url, &headers, &zip, progress, 0, "the server", "the code")?;
    set(stage, format!("Unpacking the code at {short}"));
    let tmp = partial.join("source");
    archive::unzip(&zip, &tmp, true, Some(progress))?;
    let _ = std::fs::remove_file(&zip);
    let mut name = format!("{}_{short}", stamp_from_unix(now_unix()));
    while folder.join("source").join(&name).exists() {
        name.push('_');
    }
    let dest = folder.join("source").join(&name);
    std::fs::create_dir_all(dest.parent().unwrap())?;
    std::fs::rename(&tmp, &dest).with_context(|| format!("couldn't move the code into {}", dest.display()))?;
    let (bytes, files) = dir_size(&dest);
    repo.snapshots.push(Snapshot {
        folder: name.clone(),
        branch,
        commit: commit.sha.clone(),
        commit_date: commit.date,
        message: commit.message.lines().next().unwrap_or("").to_string(),
        captured: now_iso(),
        bytes,
        files,
        readme: find_readme(&dest),
    });
    Ok(Some(name))
}

/// The readme at the top of a folder: README.md first, then any README.*, then readme files in docs/.
pub fn find_readme(dir: &Path) -> String {
    let list = |d: &Path| -> Vec<String> {
        std::fs::read_dir(d)
            .map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    for (sub, names) in [("", list(dir)), ("docs/", list(&dir.join("docs"))), (".github/", list(&dir.join(".github")))] {
        let mut readmes: Vec<&String> = names.iter().filter(|n| n.to_ascii_lowercase().starts_with("readme")).collect();
        readmes.sort_by_key(|n| {
            let l = n.to_ascii_lowercase();
            (if l == "readme.md" { 0 } else if l.ends_with(".md") || l.ends_with(".markdown") { 1 } else if l == "readme" || l.ends_with(".txt") { 2 } else { 3 }, l)
        });
        if let Some(r) = readmes.first() {
            return format!("{sub}{r}");
        }
    }
    String::new()
}

/// Download one release the user picked, by its tag (Releases, "Keep"). `repo`
/// is updated in place; the caller saves it.
#[allow(clippy::too_many_arguments)]
pub fn capture_tag(repo: &mut Repo, folder: &Path, tag: &str, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage, limits: &Limits) -> Result<Outcome> {
    let partial = folder.join(PARTIAL_DIR);
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).with_context(|| format!("couldn't make {}", partial.display()))?;
    set(stage, format!("Looking for release {tag}"));
    let rel = src.releases(RELEASES_LISTED)?.into_iter().find(|r| r.tag == tag).with_context(|| format!("the server no longer lists release {tag}"))?;
    let res = store_release(repo, folder, rel, true, src, client, progress, stage, limits);
    let _ = std::fs::remove_dir_all(&partial);
    let tag = res?;
    if tag.is_some() {
        repo.last_changed = now_iso();
    }
    Ok(Outcome { release: tag, ..Default::default() })
}

/// How many of a project's releases the app asks the server for.
pub const RELEASES_LISTED: usize = 300;

fn capture_release(repo: &mut Repo, folder: &Path, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage, limits: &Limits) -> Result<Option<String>> {
    set(stage, "Looking for the latest release");
    let Some(rel) = src.latest_release(repo.follow.prereleases)? else { return Ok(None) };
    store_release(repo, folder, rel, false, src, client, progress, stage, limits)
}

/// Delete the oldest versions of the code and releases past the limits to keep
/// (releases picked by hand aside). The names of what went, for the log.
pub fn prune(repo: &mut Repo, folder: &Path, limits: &Limits) -> Vec<String> {
    let mut gone = vec![];
    if limits.keep_source > 0 {
        while repo.snapshots.len() > limits.keep_source as usize {
            let s = repo.snapshots.remove(0);
            if std::fs::remove_dir_all(folder.join("source").join(&s.folder)).is_ok() || !folder.join("source").join(&s.folder).exists() {
                gone.push(format!("code {}", &s.commit[..s.commit.len().min(7)]));
            }
        }
    }
    if limits.keep_releases > 0 {
        loop {
            let auto: Vec<usize> = repo.releases.iter().enumerate().filter(|(_, r)| !r.picked).map(|(i, _)| i).collect();
            if auto.len() <= limits.keep_releases as usize {
                break;
            }
            let r = repo.releases.remove(auto[0]);
            let _ = std::fs::remove_dir_all(folder.join("releases").join(&r.folder));
            gone.push(format!("release {}", r.tag));
        }
    }
    gone
}

/// Download `rel` into its own folder under releases/, unless it's kept already.
#[allow(clippy::too_many_arguments)]
fn store_release(repo: &mut Repo, folder: &Path, rel: Release, picked: bool, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage, limits: &Limits) -> Result<Option<String>> {
    if rel.tag.is_empty() {
        return Ok(None);
    }
    // the same release seen before: nothing to do. The same tag published again
    // (a "nightly" that moves) is a new release with its own folder.
    if repo.releases.iter().any(|r| r.tag == rel.tag && (r.published == rel.published || rel.published.is_empty())) {
        return Ok(None);
    }
    let mut name = safe_name(&rel.tag);
    if repo.releases.iter().any(|r| r.tag == rel.tag) || folder.join("releases").join(&name).exists() {
        name = format!("{name}_{}", stamp_from_unix(now_unix()));
    }
    while folder.join("releases").join(&name).exists() {
        name.push('_');
    }
    let tmp = folder.join(PARTIAL_DIR).join("release");
    std::fs::create_dir_all(&tmp)?;
    let assets = download_assets(repo, &rel, &tmp, src, client, progress, stage, limits)?;
    std::fs::write(tmp.join("RELEASE_NOTES.md"), notes_file(&rel)).context("couldn't write the release notes")?;
    let dest = folder.join("releases").join(&name);
    std::fs::create_dir_all(dest.parent().unwrap())?;
    std::fs::rename(&tmp, &dest).with_context(|| format!("couldn't move the release into {}", dest.display()))?;
    let (bytes, _) = dir_size(&dest);
    // kept in the order they were published, so the last is the newest even when
    // an older one is picked later
    let when = |r: &ReleaseRecord| unix_from_iso(&r.published).or_else(|| unix_from_iso(&r.captured)).unwrap_or(0);
    let record = ReleaseRecord {
        folder: name,
        tag: rel.tag.clone(),
        name: rel.name.clone(),
        published: rel.published.clone(),
        prerelease: rel.prerelease,
        captured: now_iso(),
        web_url: rel.web_url.clone(),
        assets,
        bytes,
        picked,
    };
    let at = repo.releases.iter().position(|r| when(r) > when(&record)).unwrap_or(repo.releases.len());
    repo.releases.insert(at, record);
    Ok(Some(rel.tag))
}

#[allow(clippy::too_many_arguments)]
fn download_assets(repo: &Repo, rel: &Release, tmp: &Path, src: &dyn Source, client: &Client, progress: &Progress, stage: &Stage, limits: &Limits) -> Result<Vec<AssetRecord>> {
    let mut out = vec![];
    let total = rel.assets.len();
    for (i, a) in rel.assets.iter().enumerate() {
        let file = safe_name(&a.name);
        let skip = match repo.follow.files {
            Files::None => Some("only the notes are kept".to_string()),
            Files::ThisComputer if !for_this_computer(&a.name) => Some("for another system".to_string()),
            _ if limits.max_file > 0 && a.size > limits.max_file => Some(format!("over the size limit ({} MB)", limits.max_file >> 20)),
            _ => None,
        };
        if let Some(why) = skip {
            out.push(AssetRecord { name: a.name.clone(), file: String::new(), bytes: a.size, skipped: why });
            continue;
        }
        set(stage, format!("Downloading {} ({} of {total}) from {}", a.name, i + 1, rel.tag));
        let (url, headers) = src.asset_download(a);
        let n = client.download(&url, &headers, &tmp.join(&file), progress, limits.max_file, "the server", &a.name)?;
        out.push(AssetRecord { name: a.name.clone(), file, bytes: n, skipped: String::new() });
    }
    Ok(out)
}

fn notes_file(rel: &Release) -> String {
    let title = if rel.name.trim().is_empty() { rel.tag.clone() } else { rel.name.trim().to_string() };
    let mut s = format!("# {title}\n\n");
    if !rel.published.is_empty() {
        s.push_str(&format!("Tag `{}`, published {}", rel.tag, rel.published));
        if rel.prerelease {
            s.push_str(" (pre-release)");
        }
        s.push_str(".\n\n");
    }
    if !rel.web_url.is_empty() {
        s.push_str(&format!("<{}>\n\n", rel.web_url));
    }
    s.push_str(rel.notes.trim());
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_newest_and_picked_releases() {
        use crate::library::{Follow, ReleaseRecord};
        use crate::sources::{Kind, SourceRef};
        let dir = std::env::temp_dir().join(format!("reposync-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut repo = Repo::new(SourceRef { kind: Kind::GitHub, host: "github.com".into(), path: "o/n".into() }, Follow::default());
        for i in 0..4 {
            let f = format!("s{i}");
            std::fs::create_dir_all(dir.join("source").join(&f)).unwrap();
            repo.snapshots.push(Snapshot { folder: f, commit: format!("c{i}abcdef"), ..Default::default() });
            let r = format!("v{i}");
            std::fs::create_dir_all(dir.join("releases").join(&r)).unwrap();
            repo.releases.push(ReleaseRecord { folder: r.clone(), tag: r, picked: i == 0, ..Default::default() });
        }
        let gone = prune(&mut repo, &dir, &Limits { max_file: 0, keep_source: 1, keep_releases: 2 });
        assert_eq!(repo.snapshots.iter().map(|s| s.folder.as_str()).collect::<Vec<_>>(), ["s3"]);
        assert_eq!(repo.releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(), ["v0", "v2", "v3"]);
        assert!(!dir.join("source/s0").exists() && dir.join("source/s3").exists());
        assert!(!dir.join("releases/v1").exists() && dir.join("releases/v0").exists());
        assert_eq!(gone.len(), 4);
        // 0 keeps everything
        assert!(prune(&mut repo, &dir, &Limits { max_file: 0, keep_source: 0, keep_releases: 0 }).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn readmes_are_found() {
        let dir = std::env::temp_dir().join(format!("reposync-readme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        assert_eq!(find_readme(&dir), "");
        std::fs::write(dir.join("docs/README.md"), "x").unwrap();
        assert_eq!(find_readme(&dir), "docs/README.md");
        std::fs::write(dir.join("README.txt"), "x").unwrap();
        std::fs::write(dir.join("Readme.rst"), "x").unwrap();
        assert_eq!(find_readme(&dir), "README.txt");
        std::fs::write(dir.join("README.md"), "x").unwrap();
        assert_eq!(find_readme(&dir), "README.md");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
