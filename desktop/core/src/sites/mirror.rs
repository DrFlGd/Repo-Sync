//! Saving a website as it is served: the pages under a start address, and the
//! files they load from the same server, kept at their addresses' paths so the
//! built-in server can host the copy (docs/PLAN.md, "Websites as projects").
//! Used for website projects and for live captures of a repo's site. Files from
//! other servers are saved afterwards, as for any site (`vendor`).
//!
//! A capture folder holds the site's files and `_mirror/`: `manifest.json` (every
//! address and what it gave), `redirects` (the server's redirects, in the
//! `_redirects` format the server reads), `text.txt` (the pages' words, for the
//! library's search) and `README.md`.

use super::{mime, router, scan, MirrorSettings};
use crate::http::{Client, Progress};
use crate::util::{now_iso, now_unix, stamp_from_unix, write_atomic, write_json};
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use url::Url;

const MAX_FILES: usize = 5000;
const MAX_BYTES: u64 = 500 << 20;
const MAX_FILE: u64 = 100 << 20;

/// One address saved in a capture.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Entry {
    pub url: String,
    pub ctype: String,
    pub bytes: u64,
    pub sha: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Saved while the copy was open, not by the capture.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recorded: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Skipped {
    pub url: String,
    pub why: String,
}

/// `_mirror/manifest.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub start: String,
    pub origin: String,
    pub made: String,
    /// Pages by their path in the folder.
    pub pages: BTreeMap<String, Entry>,
    /// Everything else from the site's own server.
    pub files: BTreeMap<String, Entry>,
    /// (from, to, status) as the server answered.
    pub redirects: Vec<(String, String, u16)>,
    pub skipped: Vec<Skipped>,
    /// Pages the site's robots.txt asks robots not to fetch, left out.
    pub robots_skipped: u32,
    /// A digest of every page and file, which names the folder.
    pub hash: String,
}

impl Manifest {
    fn digest(&self) -> String {
        let mut h = Sha256::new();
        for (k, e) in self.pages.iter().chain(self.files.iter()) {
            h.update(k.as_bytes());
            h.update(b"\0");
            h.update(e.sha.as_bytes());
            h.update(b"\n");
        }
        for (a, b, s) in &self.redirects {
            h.update(format!("{a} {b} {s}\n").as_bytes());
        }
        super::vendor::hex(&h.finalize())
    }
}

pub fn read_manifest(root: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(root.join("_mirror").join("manifest.json")).ok()?).ok()
}

/// What a capture did.
#[derive(Debug, Default, Serialize)]
pub struct Captured {
    /// The new folder's name; None when the site hadn't changed since `previous`.
    pub folder: Option<String>,
    pub same_as: Option<String>,
    pub pages: usize,
    pub files: usize,
    pub bytes: u64,
    pub skipped: usize,
    pub title: String,
}

/// The address a website project starts from: https added when left out, the
/// fragment dropped.
pub fn normalise_start(input: &str) -> Result<Url> {
    let t = input.trim();
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let mut u = Url::parse(&with_scheme).with_context(|| format!("{t} isn't a web address"))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        bail!("{t} isn't a web address");
    }
    u.set_fragment(None);
    Ok(u)
}

/// The folder a page's links stay inside ("/docs/" for "/docs/intro.html").
fn folder_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..=i].to_string(),
        None => "/".into(),
    }
}

/// Where an address's file goes in the copy: "/" is "index.html", a page at
/// "/about" is "about.html" (served there as Pages serves it), the query is
/// dropped. None for a path that can't be a file on every computer.
pub fn local_path(path: &str, is_html: bool, ctype: &str) -> Option<String> {
    let rel = router::clean(path.trim_start_matches('/'))?;
    if rel.split('/').any(|s| s == "_mirror") {
        return None;
    }
    if rel.is_empty() || rel.ends_with('/') {
        let name = if is_html { "index.html".to_string() } else { format!("index{}", mime::extension_for(ctype)) };
        return Some(format!("{rel}{name}"));
    }
    let last = rel.rsplit('/').next().unwrap_or("");
    if is_html && !last.contains('.') {
        return Some(format!("{rel}.html"));
    }
    Some(rel)
}

fn is_html(ctype: &str) -> bool {
    let c = ctype.to_ascii_lowercase();
    c.starts_with("text/html") || c.starts_with("application/xhtml")
}

/// Addresses on the site's own server made root-relative ("https://x.org/a" is
/// "/a"), so the copy's links stay in the copy.
pub fn own_to_root(text: &str, host: &str) -> String {
    if !text.contains(host) {
        return text.to_string();
    }
    let Ok(re) = Regex::new(&format!(r"(?i)(?:https?:)?//{}(?::(?:80|443))?([/?#\x22'\s)<>`]|$)", regex::escape(host))) else { return text.to_string() };
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for c in re.captures_iter(text) {
        let m = c.get(0).unwrap();
        let next = c.get(1).map(|x| x.as_str()).unwrap_or("");
        // "//host" alone only where an address starts
        if !m.as_str().to_ascii_lowercase().starts_with("http") {
            let before = text[..m.start()].chars().last();
            if !matches!(before, None | Some('"' | '\'' | '(' | '=' | ' ' | '`' | ',' | '\n')) {
                continue;
            }
        }
        out.push_str(&text[last..m.start()]);
        if next.starts_with('/') {
            out.push_str(next);
        } else {
            out.push('/');
            out.push_str(next);
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

struct Robots {
    disallow: Vec<String>,
    allow: Vec<String>,
}

impl Robots {
    fn parse(text: &str) -> Self {
        let mut r = Robots { disallow: vec![], allow: vec![] };
        let mut ours = false;
        let mut in_group_agents = false;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(':') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "user-agent" => {
                    if !in_group_agents {
                        ours = false;
                    }
                    in_group_agents = true;
                    if v == "*" || v.to_ascii_lowercase().contains("repo-sync") {
                        ours = true;
                    }
                }
                "disallow" if ours && !v.is_empty() => {
                    in_group_agents = false;
                    r.disallow.push(v.to_string());
                }
                "allow" if ours => {
                    in_group_agents = false;
                    r.allow.push(v.to_string());
                }
                _ => in_group_agents = false,
            }
        }
        r
    }

    fn allowed(&self, path: &str) -> bool {
        let m = |rules: &[String]| rules.iter().filter(|p| robots_match(p, path)).map(|p| p.len()).max();
        match (m(&self.disallow), m(&self.allow)) {
            (Some(d), Some(a)) => a >= d,
            (Some(_), None) => false,
            _ => true,
        }
    }
}

fn robots_match(pattern: &str, path: &str) -> bool {
    let anchored = pattern.ends_with('$');
    let p = pattern.trim_end_matches('$');
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !path.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else {
            match path[pos..].find(part) {
                Some(j) => pos += j + part.len(),
                None => return false,
            }
        }
    }
    !anchored || pos == path.len() || parts.last().is_some_and(|l| l.is_empty())
}

fn get(client: &Client, url: &str) -> Result<crate::http::Fetched, String> {
    let headers: Vec<crate::http::Header> = vec![("Accept", "text/html,application/xhtml+xml,*/*;q=0.8".into())];
    client.fetch(url, &headers, MAX_FILE, false).map_err(|e| format!("{e:#}"))
}

/// Save the website at `start` into a new folder in `into` (a project's
/// `source/` or `live/`). When nothing changed since the capture in folder
/// `previous`, no folder is kept and `same_as` names it.
pub fn capture(start: &str, into: &Path, settings: &MirrorSettings, previous: Option<&Path>, client: &Client, progress: &Progress, stage: &crate::capture::Stage) -> Result<Captured> {
    let start = normalise_start(start)?;
    let host = start.host_str().unwrap_or("").to_ascii_lowercase();
    let origin = start.origin().ascii_serialization();
    let scope = if settings.same_folder { folder_of(start.path()) } else { "/".to_string() };
    let skip: Vec<String> = settings.skip.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let max_pages = if settings.max_pages == 0 { 400 } else { settings.max_pages as usize };
    let depth_limit = settings.depth;

    std::fs::create_dir_all(into)?;
    let work = into.join(format!(".partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = (|| -> Result<Captured> {
        *stage.lock().unwrap() = format!("Reading {host}");
        let robots = match client.fetch(&format!("{origin}/robots.txt"), &[], 1 << 20, true) {
            Ok(f) if f.status == 200 => Robots::parse(&String::from_utf8_lossy(&f.body)),
            _ => Robots { disallow: vec![], allow: vec![] },
        };
        let mut m = Manifest { start: start.to_string(), origin: origin.clone(), made: now_iso(), ..Default::default() };
        // (address, is a page, depth)
        let mut queue: VecDeque<(Url, bool, u32)> = VecDeque::from([(start.clone(), true, 0)]);
        let mut seen: HashSet<String> = HashSet::from([key(&start)]);
        let mut texts: Vec<String> = vec![];
        let mut total: u64 = 0;
        let mut title = String::new();
        let mut redirect_seen: HashSet<String> = HashSet::new();
        while let Some((url, page_hint, depth)) = queue.pop_front() {
            if progress.stopped() {
                bail!("Stopped.");
            }
            if m.pages.len() + m.files.len() >= MAX_FILES || total > MAX_BYTES {
                m.skipped.push(Skipped { url: url.to_string(), why: "the capture reached its size limit".into() });
                continue;
            }
            if page_hint && m.pages.len() >= max_pages {
                m.skipped.push(Skipped { url: url.to_string(), why: format!("more than {max_pages} pages") });
                continue;
            }
            *stage.lock().unwrap() = format!("Saving {}{} ({} pages, {} files)", host, url.path(), m.pages.len(), m.files.len());
            let f = match get(client, url.as_str()) {
                Ok(f) => f,
                Err(why) => {
                    m.skipped.push(Skipped { url: url.to_string(), why });
                    continue;
                }
            };
            if !(200..300).contains(&f.status) {
                m.skipped.push(Skipped { url: url.to_string(), why: format!("the server answered {}", f.status) });
                continue;
            }
            let Ok(final_url) = Url::parse(&f.url) else { continue };
            if final_url.origin().ascii_serialization() != origin {
                m.skipped.push(Skipped { url: url.to_string(), why: format!("it moved to another site ({})", final_url.host_str().unwrap_or("")) });
                continue;
            }
            if final_url.path() != url.path() && redirect_seen.insert(url.path().to_string()) {
                m.redirects.push((url.path().to_string(), final_url.path().to_string(), 301));
                if !seen.insert(key(&final_url)) && url.path() != "/" {
                    continue;
                }
            }
            let ctype = if f.ctype.is_empty() { mime::for_path(final_url.path()).to_string() } else { f.ctype.clone() };
            let html = is_html(&ctype);
            let Some(rel) = local_path(final_url.path(), html, &ctype) else {
                m.skipped.push(Skipped { url: url.to_string(), why: "its address can't be a file name".into() });
                continue;
            };
            if m.pages.contains_key(&rel) || m.files.contains_key(&rel) {
                continue;
            }
            let kind = if html { "html" } else { mime::text_kind(&ctype) };
            let mut body = f.body;
            let mut refs: Vec<(String, bool)> = vec![];
            let mut page_title = String::new();
            if !kind.is_empty() {
                let text = String::from_utf8_lossy(&body).into_owned();
                match kind {
                    "html" => {
                        page_title = scan::title(&text);
                        let base = scan::base_href(&text).and_then(|b| final_url.join(&b).ok()).unwrap_or_else(|| final_url.clone());
                        for r in scan::html_refs(&text) {
                            if let Some(u) = scan::resolve(&base, &r.url) {
                                refs.push((u.to_string(), r.kind == scan::RefKind::Page));
                            }
                        }
                        texts.push(format!("{}\n{}\n", page_title, scan::text(&text)));
                        if title.is_empty() {
                            title = page_title.clone();
                        }
                    }
                    "css" => {
                        for r in scan::css_refs(&text) {
                            if let Some(u) = scan::resolve(&final_url, &r.url) {
                                refs.push((u.to_string(), false));
                            }
                        }
                    }
                    "js" => {
                        for r in scan::js_refs(&text) {
                            if scan::is_bare(&r.url) {
                                continue;
                            }
                            if let Some(u) = scan::resolve(&final_url, &r.url) {
                                refs.push((u.to_string(), false));
                            }
                        }
                        refs.extend(scan::webpack_chunks(&text, &final_url).into_iter().map(|u| (u, false)));
                    }
                    _ => {}
                }
                if matches!(kind, "html" | "css" | "js") {
                    body = own_to_root(&text, &host).into_bytes();
                }
            }
            let sha = super::vendor::hex(&Sha256::digest(&body));
            let dest = work.join(&rel);
            if let Some(p) = dest.parent() {
                if std::fs::create_dir_all(p).is_err() {
                    m.skipped.push(Skipped { url: url.to_string(), why: "a file has the name its folder needs".into() });
                    continue;
                }
            }
            if dest.is_dir() || std::fs::write(&dest, &body).is_err() {
                m.skipped.push(Skipped { url: url.to_string(), why: "couldn't save it under its name".into() });
                continue;
            }
            total += body.len() as u64;
            let entry = Entry { url: final_url.to_string(), ctype: ctype.clone(), bytes: body.len() as u64, sha, title: page_title, recorded: false };
            if html {
                m.pages.insert(rel, entry);
            } else {
                m.files.insert(rel, entry);
            }
            for (u, is_page) in refs {
                let Ok(mut p) = Url::parse(&u) else { continue };
                p.set_fragment(None);
                if p.origin().ascii_serialization() != origin {
                    continue;
                }
                let path = p.path().to_string();
                if skip.iter().any(|s| path.starts_with(s.as_str())) {
                    continue;
                }
                // a link is a page unless its name says it's a file
                let page = is_page && {
                    let last = path.rsplit('/').next().unwrap_or("");
                    !last.contains('.') || last.ends_with(".html") || last.ends_with(".htm") || last.ends_with(".php") || last.ends_with(".aspx")
                };
                if page {
                    if depth + 1 > depth_limit || !path.starts_with(&scope) {
                        continue;
                    }
                    if !robots.allowed(&path) {
                        if seen.insert(key(&p)) {
                            m.robots_skipped += 1;
                        }
                        continue;
                    }
                }
                if seen.insert(key(&p)) {
                    queue.push_back((p, page, depth + 1));
                }
            }
        }
        if m.pages.is_empty() {
            let why = m.skipped.first().map(|s| s.why.clone()).unwrap_or_else(|| "it isn't a web page".into());
            bail!("Couldn't save {}: {why}", start);
        }
        m.hash = m.digest();
        // the same as the last capture: nothing new to keep
        if let Some(prev) = previous {
            if read_manifest(prev).is_some_and(|p| p.hash == m.hash) {
                let name = prev.file_name().map(|n| n.to_string_lossy().into_owned());
                return Ok(Captured { folder: None, same_as: name, pages: m.pages.len(), files: m.files.len(), bytes: total, skipped: m.skipped.len(), title });
            }
        }
        write_extras(&work, &m, &texts)?;
        let name = format!("{}_{}", stamp_from_unix(now_unix()), &m.hash[..7]);
        let dest = into.join(&name);
        if dest.exists() {
            bail!("{} already exists", dest.display());
        }
        std::fs::rename(&work, &dest).with_context(|| format!("couldn't name the capture {name}"))?;
        Ok(Captured { folder: Some(name), same_as: None, pages: m.pages.len(), files: m.files.len(), bytes: total, skipped: m.skipped.len(), title })
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn key(u: &Url) -> String {
    // the query is dropped from the copy, so one address per path
    format!("{}{}", u.origin().ascii_serialization(), u.path())
}

fn write_extras(root: &Path, m: &Manifest, texts: &[String]) -> Result<()> {
    let dir = root.join("_mirror");
    std::fs::create_dir_all(&dir)?;
    write_json(&dir.join("manifest.json"), &serde_json::to_value(m)?)?;
    let redirects: String = m.redirects.iter().map(|(a, b, s)| format!("{a} {b} {s}\n")).collect();
    write_atomic(&dir.join("redirects"), redirects.as_bytes())?;
    write_atomic(&dir.join("text.txt"), texts.concat().as_bytes())?;
    let readme = format!(
        "# A copy of {}\n\nSaved by Repo Sync on {}: {} pages and {} other files from {}.\n\n\
         Addresses on the site's own server were made root-relative so links stay in the copy. \
         Files from other servers (scripts, fonts) are saved separately and listed in the version's lock file.\n\n\
         `manifest.json` lists every address and what it gave; `redirects` the server's redirects.\n",
        m.start,
        m.made,
        m.pages.len(),
        m.files.len(),
        m.origin
    );
    write_atomic(&dir.join("README.md"), readme.as_bytes())?;
    Ok(())
}

/// A page or file a copy lacks, fetched from the live site while the copy is
/// open (`from` is the site's origin, `path` the request's path under the
/// copy's `base`), saved into the copy and added to its manifest. The file's
/// path in the copy, or None.
pub fn record(root: &Path, from: &str, path: &str, query: &str, base: &str, client: &Client) -> Option<String> {
    let rest = path.strip_prefix(base)?;
    router::clean(rest)?;
    let mut url = Url::parse(from).ok()?.join(&format!("/{rest}")).ok()?;
    if !query.is_empty() {
        url.set_query(Some(query));
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let f = client.fetch(url.as_str(), &[("Accept", "*/*".into())], MAX_FILE, true).ok()?;
    if !(200..300).contains(&f.status) {
        return None;
    }
    let final_url = Url::parse(&f.url).ok()?;
    if final_url.host_str()?.to_ascii_lowercase() != host {
        return None;
    }
    let ctype = if f.ctype.is_empty() { mime::for_path(final_url.path()).to_string() } else { f.ctype.clone() };
    let html = is_html(&ctype);
    // the address asked for, so the server finds it at the path it was asked at
    let rel = local_path(&format!("/{rest}"), html, &ctype)?;
    let kind = if html { "html" } else { mime::text_kind(&ctype) };
    let body = if matches!(kind, "html" | "css" | "js") { own_to_root(&String::from_utf8_lossy(&f.body), &host).into_bytes() } else { f.body };
    let dest = root.join(&rel);
    if dest.exists() {
        return None;
    }
    std::fs::create_dir_all(dest.parent()?).ok()?;
    std::fs::write(&dest, &body).ok()?;
    let mut m = read_manifest(root).unwrap_or_default();
    let entry = Entry { url: final_url.to_string(), ctype, bytes: body.len() as u64, sha: super::vendor::hex(&Sha256::digest(&body)), title: if html { scan::title(&String::from_utf8_lossy(&body)) } else { String::new() }, recorded: true };
    if html {
        m.pages.insert(rel.clone(), entry);
    } else {
        m.files.insert(rel.clone(), entry);
    }
    if let Ok(v) = serde_json::to_value(&m) { let _ = write_json(&root.join("_mirror").join("manifest.json"), &v); }
    Some(rel)
}

/// How two captures of a site differ.
#[derive(Debug, Default, Serialize)]
pub struct Comparison {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub same: usize,
    /// Pages whose words changed, with a few lines of each side.
    pub pages: Vec<PageChange>,
}

#[derive(Debug, Default, Serialize)]
pub struct PageChange {
    pub path: String,
    pub title: String,
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

/// Compare the capture (or any folder) `old` with `new`: by the manifests when
/// both have one, by the files' contents otherwise.
pub fn compare(old: &Path, new: &Path) -> Comparison {
    let a = listing(old);
    let b = listing(new);
    let mut c = Comparison::default();
    for (k, sha) in &b {
        match a.get(k) {
            None => c.added.push(k.clone()),
            Some(s) if s != sha => c.changed.push(k.clone()),
            Some(_) => c.same += 1,
        }
    }
    for k in a.keys() {
        if !b.contains_key(k) {
            c.removed.push(k.clone());
        }
    }
    for k in c.changed.iter().filter(|k| k.ends_with(".html") || k.ends_with(".htm")).take(40) {
        let (Ok(x), Ok(y)) = (std::fs::read_to_string(old.join(k)), std::fs::read_to_string(new.join(k))) else { continue };
        let lines = |t: &str| -> Vec<String> { scan::text(t).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect() };
        let (lx, ly) = (lines(&x), lines(&y));
        let sx: HashSet<&String> = lx.iter().collect();
        let sy: HashSet<&String> = ly.iter().collect();
        let removed: Vec<String> = lx.iter().filter(|l| !sy.contains(l)).take(12).cloned().collect();
        let added: Vec<String> = ly.iter().filter(|l| !sx.contains(l)).take(12).cloned().collect();
        if !removed.is_empty() || !added.is_empty() {
            c.pages.push(PageChange { path: k.clone(), title: scan::title(&y), removed, added });
        }
    }
    c
}

/// Every file under `root` but `_mirror/` and `.git/`, with a digest of each.
fn listing(root: &Path) -> BTreeMap<String, String> {
    if let Some(m) = read_manifest(root) {
        return m.pages.into_iter().chain(m.files).map(|(k, e)| (k, e.sha)).collect();
    }
    let mut out = BTreeMap::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == "_mirror" && d == root {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => {
                    if let (Ok(rel), Ok(bytes)) = (p.strip_prefix(root), std::fs::read(&p)) {
                        out.insert(rel.to_string_lossy().replace('\\', "/"), super::vendor::hex(&Sha256::digest(&bytes)));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The words of a capture's pages, for the library's search.
pub fn text_of(root: &Path) -> String {
    std::fs::read_to_string(root.join("_mirror").join("text.txt")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_become_file_names() {
        assert_eq!(local_path("/", true, "text/html").as_deref(), Some("index.html"));
        assert_eq!(local_path("/docs/", true, "text/html").as_deref(), Some("docs/index.html"));
        assert_eq!(local_path("/about", true, "text/html").as_deref(), Some("about.html"));
        assert_eq!(local_path("/a/b.css", false, "text/css").as_deref(), Some("a/b.css"));
        assert_eq!(local_path("/a%20b/c.png", false, "image/png").as_deref(), Some("a b/c.png"));
        assert_eq!(local_path("/../x", true, "text/html"), None);
        assert_eq!(local_path("/_mirror/manifest.json", false, "application/json"), None);
    }

    #[test]
    fn own_addresses_are_made_root_relative() {
        let t = r#"<a href="https://x.org/a">a</a> <a href="https://x.org">home</a> <img src="//x.org/i.png"> <a href="https://x.org.evil/b">no</a> <a href="https://y.org/c">kept</a>"#;
        let o = own_to_root(t, "x.org");
        assert!(o.contains(r#"href="/a""#) && o.contains(r#"href="/""#) && o.contains(r#"src="/i.png""#), "{o}");
        assert!(o.contains("https://x.org.evil/b") && o.contains("https://y.org/c"));
    }

    #[test]
    fn robots_rules() {
        let r = Robots::parse("User-agent: Googlebot\nDisallow: /\n\nUser-agent: *\nDisallow: /private/\nAllow: /private/ok.html\nDisallow: /*.pdf$\n");
        assert!(r.allowed("/a.html") && !r.allowed("/private/x.html") && r.allowed("/private/ok.html") && !r.allowed("/doc.pdf") && r.allowed("/doc.pdf.html"));
    }

    #[test]
    fn comparing_folders() {
        let d = std::env::temp_dir().join(format!("reposync-mirror-cmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (dir, files) in [("a", vec![("index.html", "<title>T</title><p>Hello</p><p>Old line</p>"), ("x.css", "a{}"), ("gone.txt", "1")]), ("b", vec![("index.html", "<title>T</title><p>Hello</p><p>New line</p>"), ("x.css", "a{}"), ("new.txt", "2")])] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
            for (n, t) in files {
                std::fs::write(d.join(dir).join(n), t).unwrap();
            }
        }
        let c = compare(&d.join("a"), &d.join("b"));
        assert_eq!(c.added, vec!["new.txt"]);
        assert_eq!(c.removed, vec!["gone.txt"]);
        assert_eq!(c.changed, vec!["index.html"]);
        assert_eq!(c.same, 1);
        assert_eq!(c.pages[0].removed, vec!["Old line"]);
        assert_eq!(c.pages[0].added, vec!["New line"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
