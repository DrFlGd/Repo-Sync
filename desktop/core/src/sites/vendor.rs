//! Saving the files a site loads from other servers (three.js from jsDelivr,
//! Google Fonts…) so it works offline. Files are kept once for the whole library
//! in `_library/sites/blobs/` (named by their SHA-256), and each version of a
//! site has a lock file listing the addresses it uses and the copy kept for
//! each: a version's files are frozen when it's first made available offline,
//! as the archive never replaces what it kept (docs/PLAN.md, "Working offline").

use super::{mime, scan};
use crate::http::{Client, Progress};
use crate::util::{now_iso, write_json};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use url::Url;

/// Content delivery networks whose files are saved with a site.
pub const CDN_HOSTS: &[&str] = &[
    "cdn.jsdelivr.net",
    "fastly.jsdelivr.net",
    "unpkg.com",
    "esm.sh",
    "cdnjs.cloudflare.com",
    "ga.jspm.io",
    "jspm.dev",
    "cdn.skypack.dev",
    "code.jquery.com",
    "ajax.googleapis.com",
    "stackpath.bootstrapcdn.com",
    "maxcdn.bootstrapcdn.com",
    "cdn.tailwindcss.com",
    "use.fontawesome.com",
    "cdn.plot.ly",
    "d3js.org",
    "cdn.datatables.net",
    "cdn.babylonjs.com",
    "aframe.io",
    "cdn.rawgit.com",
    "raw.githack.com",
    "rawcdn.githack.com",
    "fonts.googleapis.com",
    "fonts.gstatic.com",
    "fonts.bunny.net",
    "rsms.me",
];

/// Analytics, ads and tracking: never saved (offline they fail quietly).
const NEVER_HOSTS: &[&str] = &[
    "www.google-analytics.com",
    "google-analytics.com",
    "www.googletagmanager.com",
    "googletagmanager.com",
    "plausible.io",
    "static.cloudflareinsights.com",
    "www.clarity.ms",
    "cdn.segment.com",
    "static.hotjar.com",
    "script.hotjar.com",
    "pagead2.googlesyndication.com",
    "connect.facebook.net",
    "cdn.mxpnl.com",
    "js.sentry-cdn.com",
    "browser.sentry-cdn.com",
    "cdn.usefathom.com",
    "scripts.simpleanalyticscdn.com",
    "gc.zgo.at",
    "umami.is",
    "cloud.umami.is",
];

/// Which servers' files are kept, and which never fetched.
#[derive(Clone, Debug, Default)]
pub struct Allow {
    pub extra: Vec<String>,
    pub skip: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Saved with the site.
    Keep,
    /// Analytics and the like: left alone.
    Never,
    /// Anything else: the site needs the internet for it.
    Other,
}

impl Allow {
    pub fn class(&self, host: &str) -> Class {
        let h = host.to_ascii_lowercase();
        if self.skip.iter().any(|s| s.eq_ignore_ascii_case(&h)) || NEVER_HOSTS.contains(&h.as_str()) {
            Class::Never
        } else if CDN_HOSTS.contains(&h.as_str()) || self.extra.iter().any(|s| s.eq_ignore_ascii_case(&h)) {
            Class::Keep
        } else {
            Class::Other
        }
    }
}

/// One saved address in a lock file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Locked {
    /// The SHA-256 of the file, its name in the blob store.
    pub blob: String,
    pub ctype: String,
    pub bytes: u64,
    pub fetched: String,
    /// Saved while the site was open (not found by the scan).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recorded: bool,
}

/// A server the site needs the internet for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Needed {
    pub host: String,
    pub count: u32,
    pub example: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Failed {
    pub url: String,
    pub why: String,
}

/// A version's lock file: `<project>/site/<code|pages|live>/<folder>/lock.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Lock {
    pub made: String,
    pub refreshed: String,
    /// The servers whose addresses the server points at the saved copies.
    pub hosts: Vec<String>,
    /// Every saved address (with its query) and its copy.
    pub files: BTreeMap<String, Locked>,
    pub needs_internet: Vec<Needed>,
    pub failed: Vec<Failed>,
}

impl Lock {
    pub fn bytes(&self) -> u64 {
        self.files.values().map(|f| f.bytes).sum()
    }
    /// The copy kept for an address: exactly, or without its query.
    pub fn find(&self, url: &str) -> Option<&Locked> {
        self.files.get(url).or_else(|| url.split_once('?').and_then(|(u, _)| self.files.get(u)))
    }
}

pub fn read_lock(path: &Path) -> Option<Lock> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn write_lock(path: &Path, lock: &Lock) -> Result<()> {
    write_json(path, &serde_json::to_value(lock)?)
}

/// The library's store of saved files.
#[derive(Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(library_root: &Path) -> Self {
        Self { dir: library_root.join(crate::library::META_DIR).join("sites").join("blobs") }
    }
    pub fn path(&self, sha: &str) -> PathBuf {
        self.dir.join(sha.get(0..2).unwrap_or("00")).join(sha)
    }
    /// Keep `bytes`; their SHA-256.
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let sha = hex(&Sha256::digest(bytes));
        let p = self.path(&sha);
        if !p.is_file() {
            crate::util::write_atomic(&p, bytes)?;
        }
        Ok(sha)
    }
    pub fn get(&self, sha: &str) -> Result<Vec<u8>> {
        let p = self.path(sha);
        std::fs::read(&p).with_context(|| format!("the saved file {sha} is missing"))
    }
    /// Delete every saved file no lock file mentions (and older than `min_age`,
    /// so a file a job just saved, before its lock is written, stays); how many went.
    pub fn sweep(&self, locks: &[PathBuf], min_age: std::time::Duration) -> usize {
        let mut used = HashSet::new();
        for l in locks {
            if let Some(lock) = read_lock(l) {
                used.extend(lock.files.into_values().map(|f| f.blob));
            }
        }
        let mut gone = 0;
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return 0 };
        for d in rd.flatten() {
            let Ok(inner) = std::fs::read_dir(d.path()) else { continue };
            for f in inner.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let young = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|age| age < min_age);
                if !used.contains(&name) && !young && std::fs::remove_file(f.path()).is_ok() {
                    gone += 1;
                }
            }
            let _ = std::fs::remove_dir(d.path());
        }
        gone
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A browser's User-Agent: Google Fonts sends WOFF2 files only to browsers it knows.
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// What scanning a site's files found.
#[derive(Debug, Default)]
pub struct Found {
    /// Addresses to save (absolute).
    pub entries: BTreeSet<String>,
    /// Bare module names and the import map they resolve through.
    pub import_map: Vec<(String, String)>,
    /// Servers the site loads files from that aren't saved.
    pub needs: BTreeMap<String, (u32, String)>,
    /// Servers to point at the saved copies.
    pub hosts: BTreeSet<String>,
}

const TEXT_EXT: &[&str] = &["html", "htm", "js", "mjs", "cjs", "css", "webmanifest", "svg"];

/// Look through a site's own files for addresses on other servers.
pub fn scan_site(root: &Path, allow: &Allow) -> Found {
    let mut found = Found::default();
    let mut bare: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut files = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if depth < 12 && !name.starts_with('.') && name != "node_modules" && name != "_mirror" {
                    stack.push((e.path(), depth + 1));
                }
                continue;
            }
            let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
            if !ft.is_file() || !TEXT_EXT.contains(&ext.as_str()) || e.metadata().map(|m| m.len()).unwrap_or(0) > 8 << 20 {
                continue;
            }
            files += 1;
            if files > 20_000 {
                break;
            }
            let Ok(bytes) = std::fs::read(e.path()) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let refs = match ext.as_str() {
                "html" | "htm" => {
                    for (k, v) in scan::page_import_map(&text) {
                        if !found.import_map.iter().any(|(x, _)| *x == k) {
                            found.import_map.push((k, v));
                        }
                    }
                    scan::html_refs(&text)
                }
                "css" => scan::css_refs(&text),
                "svg" | "webmanifest" => vec![],
                _ => scan::js_refs(&text),
            };
            for r in refs {
                let u = r.url.trim();
                if (ext.starts_with("js") || ext == "mjs" || ext == "cjs" || ext.starts_with("htm")) && scan::is_bare(u) && r.kind == scan::RefKind::Script {
                    bare.insert(u.to_string());
                    continue;
                }
                if let Some(abs) = absolute(u) {
                    note(&mut found, allow, &abs, r.kind != scan::RefKind::Page);
                }
            }
            // addresses written out anywhere, for files code loads by itself
            for u in scan::absolute_urls(&text) {
                if let Ok(p) = Url::parse(&u) {
                    if allow.class(p.host_str().unwrap_or("")) == Class::Keep {
                        found.hosts.insert(p.host_str().unwrap_or("").to_ascii_lowercase());
                        if !u.ends_with('/') && !u.contains("${") && !u.contains('{') {
                            found.entries.insert(u);
                        }
                    }
                }
            }
        }
    }
    for b in bare {
        if let Some(target) = scan::resolve_specifier(&found.import_map, &b) {
            if let Some(abs) = absolute(&target) {
                note(&mut found, allow, &abs, true);
            }
        }
    }
    found
}

/// An address that's absolute or protocol-relative, as https.
fn absolute(u: &str) -> Option<String> {
    if let Some(rest) = u.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    (u.starts_with("http://") || u.starts_with("https://")).then(|| u.to_string())
}

fn note(found: &mut Found, allow: &Allow, abs: &str, resource: bool) {
    let Ok(p) = Url::parse(abs) else { return };
    let host = p.host_str().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.starts_with("127.") {
        return;
    }
    match allow.class(&host) {
        Class::Keep => {
            found.hosts.insert(host.clone());
            // a preconnect or a folder is no file to save
            if p.path() != "/" || p.query().is_some() {
                let mut u = p.clone();
                u.set_fragment(None);
                found.entries.insert(u.to_string());
            }
        }
        Class::Other if resource => {
            let e = found.needs.entry(host).or_insert((0, abs.to_string()));
            e.0 += 1;
        }
        _ => {}
    }
}

/// What making a version available offline did.
#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub saved: usize,
    pub reused: usize,
    pub bytes: u64,
    pub failed: usize,
    pub needs_internet: usize,
}

const MAX_FILES: usize = 4000;
const MAX_BYTES: u64 = 400 << 20;
const MAX_FILE: u64 = 100 << 20;

/// Save every file `root` (a version of a site) loads from the servers `allow`
/// keeps, following scripts' imports and stylesheets' fonts, into `store`, and
/// write the version's lock file. A lock that exists is kept (its files are
/// frozen) unless `refresh`; only what it lacks is fetched.
#[allow(clippy::too_many_arguments)]
pub fn make_offline(root: &Path, lock_path: &Path, store: &Store, client: &Client, allow: &Allow, refresh: bool, progress: &Progress, stage: &crate::capture::Stage) -> Result<Outcome> {
    *stage.lock().unwrap() = "Looking for files from other servers".into();
    let found = scan_site(root, allow);
    let mut lock = if refresh { Lock::default() } else { read_lock(lock_path).unwrap_or_default() };
    if lock.made.is_empty() {
        lock.made = now_iso();
    }
    if refresh {
        lock.refreshed = now_iso();
    }
    let mut out = Outcome::default();
    let mut queue: VecDeque<String> = found.entries.iter().cloned().collect();
    let mut seen: HashSet<String> = queue.iter().cloned().collect();
    let mut hosts: BTreeSet<String> = found.hosts.clone();
    hosts.extend(lock.hosts.iter().cloned());
    let mut total: u64 = 0;
    let mut n = 0;
    lock.failed.clear();
    while let Some(url) = queue.pop_front() {
        if progress.stopped() {
            anyhow::bail!("Stopped.");
        }
        n += 1;
        if n > MAX_FILES || total > MAX_BYTES {
            lock.failed.push(Failed { url, why: "too many files from other servers".into() });
            break;
        }
        let Ok(parsed) = Url::parse(&url) else { continue };
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        let (bytes, ctype) = if let Some(have) = lock.files.get(&url).cloned() {
            match store.get(&have.blob) {
                Ok(b) => {
                    out.reused += 1;
                    (b, have.ctype)
                }
                Err(_) => {
                    lock.files.remove(&url);
                    match fetch_one(client, &url, &host) {
                        Ok(x) => x,
                        Err(why) => {
                            lock.failed.push(Failed { url, why });
                            continue;
                        }
                    }
                }
            }
        } else {
            *stage.lock().unwrap() = format!("Saving {} ({} so far)", short(&url), out.saved + out.reused);
            match fetch_one(client, &url, &host) {
                Ok((b, ct)) => {
                    let sha = store.put(&b)?;
                    out.saved += 1;
                    out.bytes += b.len() as u64;
                    lock.files.insert(url.clone(), Locked { blob: sha, ctype: ct.clone(), bytes: b.len() as u64, fetched: now_iso(), recorded: false });
                    (b, ct)
                }
                Err(why) => {
                    lock.failed.push(Failed { url, why });
                    continue;
                }
            }
        };
        total += bytes.len() as u64;
        hosts.insert(host.clone());
        // what this file loads in turn
        let kind = mime::text_kind(&ctype);
        if kind.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut next: Vec<String> = vec![];
        let refs = match kind {
            "css" => scan::css_refs(&text),
            "js" => {
                next.extend(scan::webpack_chunks(&text, &parsed));
                scan::js_refs(&text)
            }
            _ => vec![],
        };
        for r in refs {
            let u = r.url.trim();
            let target = if scan::is_bare(u) {
                if kind != "js" {
                    // a CSS url(font.woff2) is relative, not a module name
                    parsed.join(u).ok().map(|x| x.to_string())
                } else {
                    scan::resolve_specifier(&found.import_map, u).and_then(|t| absolute(&t))
                }
            } else {
                scan::resolve(&parsed, u).map(|x| x.to_string())
            };
            if let Some(t) = target {
                next.push(t);
            }
        }
        for t in next {
            let Ok(p) = Url::parse(&t) else { continue };
            let h = p.host_str().unwrap_or("").to_ascii_lowercase();
            if allow.class(&h) != Class::Keep || p.path() == "/" && p.query().is_none() {
                continue;
            }
            if seen.insert(t.clone()) {
                queue.push_back(t);
            }
        }
    }
    lock.hosts = hosts.into_iter().collect();
    lock.needs_internet = found.needs.into_iter().map(|(host, (count, example))| Needed { host, count, example }).collect();
    out.failed = lock.failed.len();
    out.needs_internet = lock.needs_internet.len();
    write_lock(lock_path, &lock)?;
    Ok(out)
}

fn short(url: &str) -> String {
    let u = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    if u.chars().count() > 70 {
        format!("{}…", u.chars().take(70).collect::<String>())
    } else {
        u.to_string()
    }
}

/// Fetch one file from another server: (bytes, content type), or why not.
pub fn fetch_one(client: &Client, url: &str, host: &str) -> Result<(Vec<u8>, String), String> {
    let mut headers: Vec<crate::http::Header> = vec![("Accept", "*/*".into())];
    if host.starts_with("fonts.") || host == "rsms.me" {
        headers.push(("User-Agent", BROWSER_UA.into()));
        headers.push(("Accept", "text/css,*/*;q=0.1".into()));
    }
    match client.fetch(url, &headers, MAX_FILE, false) {
        Ok(f) if (200..300).contains(&f.status) => {
            let ct = if f.ctype.is_empty() || f.ctype.starts_with("application/octet-stream") || f.ctype.starts_with("text/plain") {
                // jsDelivr and unpkg send plain text for some files: go by the name
                let by_name = mime::for_path(url.split(['?', '#']).next().unwrap_or(url));
                if by_name == "application/octet-stream" && !f.ctype.is_empty() {
                    f.ctype.clone()
                } else {
                    by_name.to_string()
                }
            } else {
                f.ctype.clone()
            };
            Ok((f.body, ct))
        }
        Ok(f) => Err(format!("the server answered {}", f.status)),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// Save one file a page asked for while it was open (a miss on the saved-files
/// route): fetched, stored and added to the lock. None when the address isn't on
/// a server that's kept, or it couldn't be fetched.
pub fn record(url: &str, lock_path: &Path, store: &Store, client: &Client, allow: &Allow) -> Option<Locked> {
    let p = Url::parse(url).ok()?;
    let host = p.host_str()?.to_ascii_lowercase();
    if allow.class(&host) != Class::Keep {
        return None;
    }
    let (bytes, ctype) = fetch_quick(client, url, &host).ok()?;
    let sha = store.put(&bytes).ok()?;
    let locked = Locked { blob: sha, ctype, bytes: bytes.len() as u64, fetched: now_iso(), recorded: true };
    let mut lock = read_lock(lock_path).unwrap_or_default();
    if lock.made.is_empty() {
        lock.made = now_iso();
    }
    if !lock.hosts.contains(&host) {
        lock.hosts.push(host);
    }
    lock.files.insert(url.to_string(), locked.clone());
    write_lock(lock_path, &lock).ok()?;
    Some(locked)
}

fn fetch_quick(client: &Client, url: &str, host: &str) -> Result<(Vec<u8>, String), String> {
    let mut headers: Vec<crate::http::Header> = vec![];
    if host.starts_with("fonts.") {
        headers.push(("User-Agent", BROWSER_UA.into()));
    }
    match client.fetch(url, &headers, MAX_FILE, true) {
        Ok(f) if (200..300).contains(&f.status) => {
            let ct = if f.ctype.is_empty() { mime::for_path(url.split('?').next().unwrap_or(url)).to_string() } else { f.ctype };
            Ok((f.body, ct))
        }
        Ok(f) => Err(format!("{}", f.status)),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_s_other_servers_are_found() {
        let d = std::env::temp_dir().join(format!("reposync-vendor-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("js")).unwrap();
        std::fs::write(
            d.join("index.html"),
            r#"<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Chakra+Petch:wght@500&amp;display=swap">
<script type="importmap">{"imports":{"three":"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js","three/addons/":"https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/"}}</script>
<script async src="https://www.googletagmanager.com/gtag/js?id=G-1"></script><script src="https://api.example.com/sdk.js"></script>
<a href="https://github.com/o/r">source</a>"#,
        )
        .unwrap();
        std::fs::write(d.join("js/three.js"), "export * from 'https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js';").unwrap();
        std::fs::write(d.join("js/guns.js"), "import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js'; const base = `https://cdn.jsdelivr.net/gh/o/r@${sha}/`;").unwrap();
        let f = scan_site(&d, &Allow::default());
        assert!(f.entries.contains("https://fonts.googleapis.com/css2?family=Chakra+Petch:wght@500&display=swap"), "{:?}", f.entries);
        assert!(f.entries.contains("https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js"));
        assert!(f.entries.contains("https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/loaders/GLTFLoader.js"), "a bare import resolves through the import map");
        assert!(!f.entries.iter().any(|u| u.contains("googletagmanager") || u.ends_with(".net/") || u.contains("${")));
        assert!(f.hosts.contains("fonts.googleapis.com") && f.hosts.contains("cdn.jsdelivr.net"));
        assert!(f.needs.contains_key("api.example.com") && !f.needs.contains_key("github.com") && !f.needs.contains_key("www.googletagmanager.com"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_store_keeps_one_copy_and_sweeps_the_unused() {
        let d = std::env::temp_dir().join(format!("reposync-vendor-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let s = Store::new(&d);
        let a = s.put(b"one").unwrap();
        assert_eq!(s.put(b"one").unwrap(), a);
        let b = s.put(b"two").unwrap();
        assert_eq!(s.get(&a).unwrap(), b"one");
        let lock = d.join("p/site/code/v1/lock.json");
        let mut l = Lock::default();
        l.files.insert("https://x/a".into(), Locked { blob: a.clone(), ..Default::default() });
        write_lock(&lock, &l).unwrap();
        assert_eq!(s.sweep(&[lock.clone()], std::time::Duration::from_secs(3600)), 0);
        assert_eq!(s.sweep(&[lock], std::time::Duration::ZERO), 1);
        assert!(s.path(&a).is_file() && !s.path(&b).exists());
        assert!(l.find("https://x/a?v=2").is_some() && l.find("https://x/b").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
