//! The app's website commands: hosting a project's site on the built-in server,
//! saving the files it loads from other servers, capturing live websites,
//! building and exporting (docs/PLAN.md, "Hosting websites").

use super::{App, JobKind};
use crate::capture::{self, Outcome};
use crate::http::Progress;
use crate::library::{Library, Repo, Snapshot};
use crate::sites::server::{self, Listed, LogEntry, Server, Sites, View};
use crate::sites::{self, build, export, mirror, router::Hide, vendor, Coll, SiteInfo, SiteKind, SiteSettings};
use crate::sources::Kind;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

/// What a website job does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "lowercase")]
pub enum SiteTask {
    /// Save the files a version loads from other servers.
    Offline { coll: Coll, folder: String, refresh: bool },
    /// Capture the live website now.
    Capture,
    /// Build a version on this computer.
    Build { coll: Coll, folder: String },
    /// Package a version as a repo (and push it).
    Export {
        coll: Coll,
        folder: String,
        options: export::Options,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        push: String,
    },
}

#[derive(Default)]
pub struct SiteState {
    server: Mutex<Option<Arc<Server>>>,
    /// What versions captured before sites were detected hold, worked out once.
    detected: Mutex<HashMap<PathBuf, SiteInfo>>,
}

struct Host(Weak<App>);

impl Sites for Host {
    fn view(&self, label: &str) -> Option<View> {
        self.0.upgrade()?.site_view(label)
    }
    fn list(&self) -> Vec<Listed> {
        self.0.upgrade().map(|a| a.site_list()).unwrap_or_default()
    }
    fn client(&self) -> Arc<crate::http::Client> {
        self.0.upgrade().map(|a| a.client.clone()).unwrap_or_default()
    }
}

/// How one version is served.
struct Served {
    root: PathBuf,
    base: String,
    hide: Hide,
    spa: bool,
    markdown: bool,
    kind: SiteKind,
    /// The path to open ("/vectorclad/").
    entry: String,
    built: bool,
    /// Why it can't be served as it is ("" when it can).
    why_not: String,
    info: SiteInfo,
}

pub fn label_of(r: &Repo) -> String {
    if r.site.label.trim().is_empty() {
        sites::default_label(&r.source)
    } else {
        r.site.label.trim().to_ascii_lowercase()
    }
}

fn list_of(r: &Repo, coll: Coll) -> &Vec<Snapshot> {
    match coll {
        Coll::Code => &r.snapshots,
        Coll::Pages => &r.pages,
        Coll::Live => &r.live,
    }
}

/// `<project>/site/<code|pages|live>/<folder>/`: the version's lock, build and notes.
fn work_dir(l: &Library, r: &Repo, coll: Coll, folder: &str) -> PathBuf {
    l.folder_of(&r.id()).join("site").join(coll.key()).join(folder)
}

fn version_dir(l: &Library, r: &Repo, coll: Coll, folder: &str) -> PathBuf {
    l.folder_of(&r.id()).join(coll.dir()).join(folder)
}

/// Every lock file in the library, for sweeping the store.
fn all_locks(l: &Library) -> Vec<PathBuf> {
    let mut out = vec![];
    for id in l.repos.keys() {
        for coll in [Coll::Code, Coll::Pages, Coll::Live] {
            let d = l.folder_of(id).join("site").join(coll.key());
            if let Ok(rd) = std::fs::read_dir(&d) {
                out.extend(rd.flatten().map(|e| e.path().join("lock.json")).filter(|p| p.is_file()));
            }
        }
    }
    out
}

/// Delete saved files nothing uses any more.
pub fn sweep(l: &Library) -> usize {
    vendor::Store::new(&l.root).sweep(&all_locks(l), std::time::Duration::from_secs(15 * 60))
}

fn allow_of(s: &SiteSettings) -> vendor::Allow {
    let mut extra = s.extra_hosts.clone();
    extra.extend(s.mirror.hosts.iter().cloned());
    vendor::Allow { extra, skip: s.skip_hosts.clone() }
}

fn origin_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().map(|u| u.origin().ascii_serialization()).filter(|o| o != "null")
}

impl App {
    pub(super) fn site_state_new() -> SiteState {
        SiteState::default()
    }

    /// What a version holds as a website: from repo.json, or worked out now for a
    /// version captured before the app knew.
    fn info_for(&self, l: &Library, r: &Repo, coll: Coll, s: &Snapshot) -> SiteInfo {
        if let Some(i) = &s.site {
            return i.clone();
        }
        let dir = version_dir(l, r, coll, &s.folder);
        if let Some(i) = self.site.detected.lock().unwrap().get(&dir) {
            return i.clone();
        }
        let info = match coll {
            Coll::Pages => sites::detect::detect_branch(&dir, &r.source),
            _ => {
                let hints = sites::detect::Hints { has_pages: r.details.has_pages, pages: r.details.pages.clone(), default_branch: r.details.default_branch.clone(), branch: s.branch.clone() };
                sites::detect::detect(&dir, &r.source, &hints)
            }
        };
        self.site.detected.lock().unwrap().insert(dir, info.clone());
        info
    }

    fn served(&self, l: &Library, r: &Repo, coll: Coll, s: &Snapshot) -> Served {
        let info = self.info_for(l, r, coll, s);
        let set = &r.site;
        let dir = version_dir(l, r, coll, &s.folder);
        let mut kind = info.kind;
        match set.kind.as_str() {
            "static" => kind = SiteKind::Static,
            "jekyll-passthrough" => kind = SiteKind::JekyllPassthrough,
            "build" if coll == Coll::Code => kind = SiteKind::Build,
            _ => {}
        }
        if coll == Coll::Live {
            kind = SiteKind::Mirror;
        }
        let root_rel = if !set.root.trim().is_empty() && coll == Coll::Code { set.root.trim().trim_matches('/').to_string() } else { info.root.clone() };
        let mut root = match crate::util::safe_rel(Path::new(&root_rel)) {
            Some(r) if !root_rel.is_empty() => dir.join(r),
            _ => dir.clone(),
        };
        let mut base = if !set.base.trim().is_empty() && kind != SiteKind::Mirror {
            let b = set.base.trim().trim_matches('/');
            if b.is_empty() {
                "/".to_string()
            } else {
                format!("/{b}/")
            }
        } else if info.base.is_empty() {
            "/".to_string()
        } else {
            info.base.clone()
        };
        let mut hide = match kind {
            SiteKind::Mirror => Hide::Mirror,
            SiteKind::JekyllPassthrough | SiteKind::Jekyll => Hide::Jekyll,
            _ => Hide::Artifact,
        };
        let mut built = false;
        let mut why_not = String::new();
        if matches!(kind, SiteKind::Build | SiteKind::Jekyll) {
            if let Some(b) = build::built_root(&work_dir(l, r, coll, &s.folder)) {
                root = b;
                built = true;
                hide = Hide::Artifact;
            } else if kind == SiteKind::Build {
                why_not = "This site is built when it's published. Build it on this computer (Site tab), or open the published branch or a live capture.".into();
            }
        }
        if kind == SiteKind::Branch {
            why_not = "This version holds the site's source; the built site is on its published branch.".into();
        }
        if kind == SiteKind::None && coll == Coll::Code && set.enabled != Some(true) {
            why_not = "No website was found in this version.".into();
        }
        if !root.is_dir() {
            why_not = format!("The site's folder ({}) isn't in this version.", if root_rel.is_empty() { "the top" } else { &root_rel });
        }
        let spa = match set.spa_fallback.as_str() {
            "index" => true,
            "none" => false,
            _ => info.spa,
        };
        let markdown = matches!(kind, SiteKind::JekyllPassthrough | SiteKind::Jekyll | SiteKind::None) && !built;
        let entry = if kind == SiteKind::Mirror {
            base = "/".into();
            format!("/{}", info.entry.trim_start_matches('/'))
        } else {
            base.clone()
        };
        Served { root, base, hide, spa, markdown, kind, entry, built, why_not, info }
    }

    /// The version served at a site's main address: the one pinned, else the
    /// newest that can be served (a built or published branch over source that
    /// needs building), else the newest live capture.
    fn main_version(&self, l: &Library, r: &Repo) -> Option<(Coll, Snapshot)> {
        if let Some((c, f)) = r.site.pin.split_once('/') {
            if let Some(coll) = Coll::from_key(c) {
                if let Some(s) = list_of(r, coll).iter().find(|s| s.folder == f) {
                    return Some((coll, s.clone()));
                }
            }
        }
        let ok = |coll: Coll, s: &Snapshot| self.served(l, r, coll, s).why_not.is_empty();
        if let Some(s) = r.snapshots.last().filter(|s| ok(Coll::Code, s)) {
            return Some((Coll::Code, s.clone()));
        }
        if let Some(s) = r.pages.last().filter(|s| ok(Coll::Pages, s)) {
            return Some((Coll::Pages, s.clone()));
        }
        if let Some(s) = r.live.last() {
            return Some((Coll::Live, s.clone()));
        }
        None
    }

    fn is_site(&self, l: &Library, r: &Repo) -> bool {
        if r.source.kind == Kind::Web || !r.pages.is_empty() || !r.live.is_empty() || r.site.enabled == Some(true) {
            return r.site.enabled != Some(false);
        }
        r.site.enabled != Some(false) && r.latest_snapshot().is_some_and(|s| self.info_for(l, r, Coll::Code, s).kind.is_site())
    }

    fn view_of(&self, l: &Library, r: &Repo, coll: Coll, s: &Snapshot) -> Option<View> {
        let sv = self.served(l, r, coll, s);
        if !sv.why_not.is_empty() && !(sv.kind == SiteKind::None && sv.root.is_dir()) {
            return None;
        }
        let record_from = if sv.kind == SiteKind::Mirror && r.site.recording {
            let url = if r.source.kind == Kind::Web || coll == Coll::Live {
                mirror::read_manifest(&version_dir(l, r, coll, &s.folder)).map(|m| m.origin).or_else(|| origin_of(&capture::live_url(r)))
            } else {
                None
            };
            url.filter(|u| !u.is_empty())
        } else {
            None
        };
        let title = if r.details.name.is_empty() { r.source.name().to_string() } else { r.details.name.clone() };
        Some(View {
            project: r.id(),
            title,
            root: sv.root,
            base: sv.base,
            hide: sv.hide,
            spa: sv.spa,
            markdown: sv.markdown,
            lock: Some(work_dir(l, r, coll, &s.folder).join("lock.json")),
            store: vendor::Store::new(&l.root),
            allow: allow_of(&r.site),
            headers: r.site.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            block_internet: r.site.block_internet,
            save_misses: r.site.recording,
            record_from,
        })
    }

    /// The site a host name's label names: "<label>" (the main version), or
    /// "<code>.<label>", "p-<code>.<label>", "l-<code>.<label>" (one version).
    pub(super) fn site_view(&self, label: &str) -> Option<View> {
        let g = self.lib.lock().unwrap();
        let l = g.as_ref()?;
        let label = label.to_ascii_lowercase();
        for r in l.repos.values() {
            let mine = label_of(r);
            if mine == label {
                if !self.is_site(l, r) {
                    return None;
                }
                let (coll, s) = self.main_version(l, r)?;
                return self.view_of(l, r, coll, &s);
            }
            if let Some(code) = label.strip_suffix(&format!(".{mine}")) {
                for coll in [Coll::Code, Coll::Pages, Coll::Live] {
                    let Some(c) = code.strip_prefix(coll.prefix()) else { continue };
                    if coll == Coll::Code && (code.starts_with("p-") || code.starts_with("l-")) {
                        continue;
                    }
                    if let Some(s) = list_of(r, coll).iter().rev().find(|s| sites::version_code(&s.folder) == c) {
                        return self.view_of(l, r, coll, s);
                    }
                }
            }
        }
        None
    }

    fn site_list(&self) -> Vec<Listed> {
        let g = self.lib.lock().unwrap();
        let Some(l) = g.as_ref() else { return vec![] };
        l.repos
            .values()
            .filter(|r| self.is_site(l, r))
            .filter_map(|r| {
                let (coll, s) = self.main_version(l, r)?;
                let sv = self.served(l, r, coll, &s);
                Some(Listed { label: label_of(r), title: if r.details.name.is_empty() { r.source.name().to_string() } else { r.details.name.clone() }, base: sv.entry })
            })
            .collect()
    }

    /// The server, started when first needed.
    pub(super) fn ensure_server(&self) -> Result<Arc<Server>> {
        let mut g = self.site.server.lock().unwrap();
        if let Some(s) = g.as_ref() {
            return Ok(s.clone());
        }
        let wanted = self.config.lock().unwrap().site_port;
        let s = server::start(wanted, true, Arc::new(Host(self.me.clone())))?;
        if wanted != Some(s.port) {
            let mut c = self.config.lock().unwrap();
            c.site_port = Some(s.port);
            let _ = c.save(&self.paths.config_dir.join("config.json"));
        }
        *g = Some(s.clone());
        Ok(s)
    }

    pub(super) fn server_port(&self) -> Option<u16> {
        self.site.server.lock().unwrap().as_ref().map(|s| s.port)
    }

    fn address(port: u16, host_label: &str, path: &str) -> String {
        format!("http://{host_label}.localhost:{port}{path}")
    }

    fn version_label(r: &Repo, coll: Coll, folder: &str) -> String {
        format!("{}{}.{}", coll.prefix(), sites::version_code(folder), label_of(r))
    }

    pub(super) fn site_dispatch(&self, cmd: &str, a: &Value) -> Result<Option<Value>> {
        let id = || a["id"].as_str().map(String::from).context("no project given");
        let coll = || a["coll"].as_str().and_then(Coll::from_key).context("no list of versions given");
        let folder = || a["folder"].as_str().filter(|f| !f.is_empty()).map(String::from).context("no version given");
        Ok(Some(match cmd {
            "site_info" => self.site_info(&id()?)?,
            "site_set" => self.site_set(&id()?, &a["site"])?,
            "site_open" => self.site_open(&id()?, a["coll"].as_str().and_then(Coll::from_key), a["folder"].as_str())?,
            "sites" => self.sites_json()?,
            "site_log" => {
                let id = id()?;
                let log: Vec<LogEntry> = self.site.server.lock().unwrap().as_ref().map(|s| s.log_of(&id)).unwrap_or_default();
                json!(log)
            }
            "site_offline" => {
                let id = id()?;
                let task = SiteTask::Offline { coll: coll()?, folder: folder()?, refresh: a["refresh"].as_bool().unwrap_or(false) };
                json!({ "job": self.enqueue(JobKind::Site { id: id.clone(), task }, id, "you") })
            }
            "site_capture" => {
                let id = id()?;
                let web = self.with_lib(|l| Ok(l.repos.get(&id).context("That project isn't in the library.")?.source.kind == Kind::Web))?;
                let kind = if web { JobKind::Check { id: id.clone() } } else { JobKind::Site { id: id.clone(), task: SiteTask::Capture } };
                json!({ "job": self.enqueue(kind, id, "you") })
            }
            "site_build" => {
                let id = id()?;
                let (coll, folder) = (coll()?, folder()?);
                if a["approve"].as_bool() == Some(true) {
                    self.with_lib(|l| {
                        let mut r = l.repos.get(&id).cloned().context("That project isn't in the library.")?;
                        r.site.build.approved = true;
                        l.save_repo(&r)?;
                        l.repos.insert(id.clone(), r);
                        Ok(())
                    })?;
                }
                let approved = self.with_lib(|l| Ok(l.repos.get(&id).is_some_and(|r| r.site.build.approved)))?;
                if !approved {
                    bail!("Building runs this project's own code on this computer. Agree to it first.");
                }
                json!({ "job": self.enqueue(JobKind::Site { id: id.clone(), task: SiteTask::Build { coll, folder } }, id, "you") })
            }
            "site_export" => {
                let id = id()?;
                let options: export::Options = serde_json::from_value(a["options"].clone()).context("those export settings can't be read")?;
                let push = a["push"].as_str().unwrap_or("").trim().to_string();
                let task = SiteTask::Export { coll: coll()?, folder: folder()?, options, push };
                json!({ "job": self.enqueue(JobKind::Site { id: id.clone(), task }, id, "you") })
            }
            "site_compare" => {
                let id = id()?;
                let pick = |v: &Value| -> Result<(Coll, String)> { Ok((v["coll"].as_str().and_then(Coll::from_key).context("no list of versions given")?, v["folder"].as_str().context("no version given")?.to_string())) };
                let (ca, fa) = pick(&a["a"])?;
                let (cb, fb) = pick(&a["b"])?;
                let (da, db) = self.with_lib(|l| {
                    let r = l.repos.get(&id).context("That project isn't in the library.")?;
                    let root = |c: Coll, f: &str| -> Result<PathBuf> {
                        let s = list_of(r, c).iter().find(|s| s.folder == f).context("That version isn't in the library.")?;
                        Ok(self.served(l, r, c, s).root)
                    };
                    Ok((root(ca, &fa)?, root(cb, &fb)?))
                })?;
                json!(mirror::compare(&da, &db))
            }
            _ => return Ok(None),
        }))
    }

    fn site_info(&self, id: &str) -> Result<Value> {
        let port = self.server_port();
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let main = self.main_version(l, r);
            let label = label_of(r);
            let mut versions = vec![];
            for coll in [Coll::Code, Coll::Pages, Coll::Live] {
                for s in list_of(r, coll).iter().rev() {
                    let sv = self.served(l, r, coll, s);
                    let wd = work_dir(l, r, coll, &s.folder);
                    let lock = vendor::read_lock(&wd.join("lock.json"));
                    let rec = build::read_record(&wd);
                    let plan = if matches!(sv.kind, SiteKind::Build | SiteKind::Jekyll) { build::plan(sv.info.build.as_ref(), &r.site.build) } else { None };
                    let vlabel = Self::version_label(r, coll, &s.folder);
                    versions.push(json!({
                        "coll": coll.key(),
                        "folder": s.folder,
                        "code": sites::version_code(&s.folder),
                        "commit": s.commit,
                        "branch": s.branch,
                        "message": s.message,
                        "captured": s.captured,
                        "bytes": s.bytes,
                        "kind": sv.kind,
                        "kind_label": sv.kind.label(),
                        "servable": sv.why_not.is_empty(),
                        "why_not": sv.why_not,
                        "built": sv.built,
                        "root": sv.info.root,
                        "base": sv.base,
                        "entry": sv.entry,
                        "spa": sv.spa,
                        "evidence": sv.info.evidence,
                        "warnings": sv.info.warnings,
                        "main": main.as_ref().is_some_and(|(c, m)| *c == coll && m.folder == s.folder),
                        "label": vlabel,
                        "url": port.map(|p| Self::address(p, &vlabel, &sv.entry)),
                        "offline": lock.map(|k| json!({
                            "files": k.files.len(),
                            "bytes": k.bytes(),
                            "hosts": k.hosts,
                            "made": k.made,
                            "refreshed": k.refreshed,
                            "failed": k.failed,
                            "needs_internet": k.needs_internet,
                            "recorded": k.files.values().filter(|f| f.recorded).count(),
                        })),
                        "build": rec,
                        "build_log": if rec_exists(&wd) { build::log_tail(&wd, 6000) } else { String::new() },
                        "plan": plan,
                    }));
                }
            }
            let main_json = main.as_ref().map(|(c, s)| {
                let sv = self.served(l, r, *c, s);
                json!({ "coll": c.key(), "folder": s.folder, "entry": sv.entry, "kind": sv.kind, "kind_label": sv.kind.label() })
            });
            let entry = main.as_ref().map(|(c, s)| self.served(l, r, *c, s).entry).unwrap_or_else(|| "/".into());
            Ok(json!({
                "is_site": self.is_site(l, r),
                "website": r.source.kind == Kind::Web,
                "label": label,
                "port": port,
                "url": port.filter(|_| main.is_some()).map(|p| Self::address(p, &label, &entry)),
                "main": main_json,
                "settings": r.site,
                "live_url": capture::live_url(r),
                "site_branch": capture::site_branch(r),
                "follow_live": r.follow.live,
                "follow_site_branch": r.follow.site_branch,
                "pages": r.details.pages,
                "versions": versions,
            }))
        })
    }

    fn site_set(&self, id: &str, v: &Value) -> Result<Value> {
        let s: SiteSettings = serde_json::from_value(v.clone()).context("those website settings can't be read")?;
        let label = s.label.trim().to_ascii_lowercase();
        if !label.is_empty() {
            let ok = label.len() <= 200 && label.split('.').all(|p| !p.is_empty() && p.len() <= 63 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !p.starts_with('-') && !p.ends_with('-'));
            if !ok {
                bail!("The site's name can have letters, digits, '-' and '.' only.");
            }
        }
        if !s.live_url.trim().is_empty() {
            mirror::normalise_start(&s.live_url)?;
        }
        self.with_lib(|l| {
            let mine = l.repos.get(id).context("That project isn't in the library.")?;
            let new_label = if label.is_empty() { sites::default_label(&mine.source) } else { label.clone() };
            if l.repos.values().any(|r| r.id() != id && label_of(r) == new_label) {
                bail!("Another project's site is called {new_label}. Choose another name.");
            }
            Ok(())
        })?;
        let mut s = s;
        s.label = label;
        self.edit(id, |r| {
            r.site = s;
            Ok(())
        })?;
        self.site_info(id)
    }

    fn site_open(&self, id: &str, coll: Option<Coll>, folder: Option<&str>) -> Result<Value> {
        let srv = self.ensure_server()?;
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let (host_label, sv) = match (coll, folder) {
                (Some(c), Some(f)) => {
                    let s = list_of(r, c).iter().find(|s| s.folder == f).context("That version isn't in the library.")?;
                    (Self::version_label(r, c, f), self.served(l, r, c, s))
                }
                _ => {
                    let (c, s) = self.main_version(l, r).context("No version of this project can be shown as a website yet.")?;
                    (label_of(r), self.served(l, r, c, &s))
                }
            };
            if !sv.why_not.is_empty() && sv.kind != SiteKind::None {
                bail!(sv.why_not);
            }
            Ok(json!({ "url": Self::address(srv.port, &host_label, &sv.entry), "port": srv.port }))
        })
    }

    fn sites_json(&self) -> Result<Value> {
        let port = self.ensure_server()?.port;
        let list = self.site_list();
        Ok(json!({
            "port": port,
            "index": format!("http://localhost:{port}/"),
            "sites": list.iter().map(|s| json!({ "label": s.label, "title": s.title, "url": Self::address(port, &s.label, &s.base) })).collect::<Vec<_>>(),
        }))
    }

    /// After a check: save the files new versions load from other servers
    /// (when the project's site settings say to). Problems become warnings.
    pub(super) fn after_capture(&self, id: &str, out: &mut Outcome, progress: &Progress, stage: &capture::Stage) {
        let todo: Vec<(Coll, String)> = [(Coll::Code, out.snapshot.clone()), (Coll::Pages, out.pages.clone()), (Coll::Live, out.live.clone())].into_iter().filter_map(|(c, f)| f.map(|f| (c, f))).collect();
        for (coll, folder) in todo {
            if progress.stopped() {
                return;
            }
            let go = self.with_lib(|l| Ok(l.repos.get(id).is_some_and(|r| r.site.offline && r.site.enabled != Some(false) && self.is_site(l, r)))).unwrap_or(false);
            if !go {
                continue;
            }
            match self.make_offline(id, coll, &folder, false, progress, stage) {
                Ok(Some(o)) if o.failed > 0 => out.warnings.push(format!("Offline: {} files from other servers couldn't be saved", o.failed)),
                Ok(_) => {}
                Err(_) if progress.stopped() => return,
                Err(e) => out.warnings.push(format!("Offline: {e:#}")),
            }
        }
    }

    fn make_offline(&self, id: &str, coll: Coll, folder: &str, refresh: bool, progress: &Progress, stage: &capture::Stage) -> Result<Option<vendor::Outcome>> {
        let (root, lock, store, allow, servable) = self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let s = list_of(r, coll).iter().find(|s| s.folder == folder).context("That version isn't in the library.")?;
            let sv = self.served(l, r, coll, s);
            Ok((sv.root, work_dir(l, r, coll, folder).join("lock.json"), vendor::Store::new(&l.root), allow_of(&r.site), sv.why_not.is_empty()))
        })?;
        if !servable {
            return Ok(None);
        }
        Ok(Some(vendor::make_offline(&root, &lock, &store, &self.client, &allow, refresh, progress, stage)?))
    }

    pub(super) fn run_site(&self, id: &str, task: &SiteTask, progress: &Progress, stage: &capture::Stage) -> Result<Outcome> {
        match task {
            SiteTask::Offline { coll, folder, refresh } => {
                let o = self.make_offline(id, *coll, folder, *refresh, progress, stage)?.context("This version can't be shown as a website as it is, so there's nothing to save for it.")?;
                let mut note = format!("Saved for offline use: {} new, {} kept before ({} MB)", o.saved, o.reused, (o.bytes as f64 / 1048576.0 * 10.0).round() / 10.0);
                if o.needs_internet > 0 {
                    note.push_str(&format!("; {} servers still need the internet", o.needs_internet));
                }
                let mut out = Outcome { note: Some(note), ..Default::default() };
                if o.failed > 0 {
                    out.warnings.push(format!("{} files couldn't be saved", o.failed));
                }
                Ok(out)
            }
            SiteTask::Capture => {
                let (mut repo, folder) = self.with_lib(|l| Ok((l.repos.get(id).cloned().context("That project isn't in the library any more.")?, l.folder_of(id))))?;
                let url = capture::live_url(&repo);
                if url.is_empty() {
                    bail!("This project has no live address to capture. Add it in the Site tab's settings.");
                }
                let _ = std::fs::create_dir_all(folder.join(crate::library::PARTIAL_DIR));
                let name = capture::capture_website(&mut repo, &folder, Coll::Live, &url, &self.client, progress, stage)?;
                self.with_lib(|l| {
                    let Some(cur) = l.repos.get(id) else { return Ok(()) };
                    let mut r = cur.clone();
                    r.live = repo.live.clone();
                    if name.is_some() {
                        r.last_changed = crate::util::now_iso();
                    }
                    l.save_repo(&r)?;
                    l.repos.insert(id.to_string(), r);
                    Ok(())
                })?;
                let mut out = Outcome { live: name.clone(), ..Default::default() };
                self.after_capture(id, &mut out, progress, stage);
                out.note = Some(match &name {
                    Some(n) => format!("Captured the live website ({n})"),
                    None => "The live website hasn't changed since the last capture".into(),
                });
                Ok(out)
            }
            SiteTask::Build { coll, folder } => {
                let (src, into, plan) = self.with_lib(|l| {
                    let r = l.repos.get(id).context("That project isn't in the library.")?;
                    if !r.site.build.approved {
                        bail!("Building runs this project's own code on this computer. Agree to it first.");
                    }
                    let s = list_of(r, *coll).iter().find(|s| &s.folder == folder).context("That version isn't in the library.")?;
                    let info = self.info_for(l, r, *coll, s);
                    let plan = build::plan(info.build.as_ref(), &r.site.build).context("There's no build command for this site. Set one in the Site tab's settings.")?;
                    Ok((version_dir(l, r, *coll, folder), work_dir(l, r, *coll, folder), plan))
                })?;
                let rec = build::run(&src, &into, &plan, progress, stage)?;
                if !rec.ok {
                    bail!(rec.why);
                }
                let mut out = Outcome { note: Some(format!("Built in {} s", rec.seconds)), ..Default::default() };
                let offline = self.with_lib(|l| Ok(l.repos.get(id).is_some_and(|r| r.site.offline))).unwrap_or(false);
                if offline {
                    if let Err(e) = self.make_offline(id, *coll, folder, false, progress, stage) {
                        out.warnings.push(format!("Offline: {e:#}"));
                    }
                }
                Ok(out)
            }
            SiteTask::Export { coll, folder, options, push } => {
                let (root, lock, store, title, source, version) = self.with_lib(|l| {
                    let r = l.repos.get(id).context("That project isn't in the library.")?;
                    let s = list_of(r, *coll).iter().find(|s| &s.folder == folder).context("That version isn't in the library.")?;
                    let sv = self.served(l, r, *coll, s);
                    let lock = vendor::read_lock(&work_dir(l, r, *coll, folder).join("lock.json"));
                    let title = if r.details.name.is_empty() { r.source.name().to_string() } else { r.details.name.clone() };
                    let source = if r.source.kind == Kind::Web { r.site.live_url.clone() } else { r.source.web_url() };
                    let version = format!("{} {} ({})", match coll {
                        Coll::Code => if r.source.kind == Kind::Web { "capture" } else { "code" },
                        Coll::Pages => "published branch",
                        Coll::Live => "live capture",
                    }, sites::version_code(folder), s.captured);
                    Ok((sv.root, lock, vendor::Store::new(&l.root), title, source, version))
                })?;
                let mut opts = options.clone();
                if opts.title.is_empty() {
                    opts.title = title;
                }
                opts.source = source;
                opts.version = version;
                if !push.is_empty() {
                    opts.format = export::Format::Git;
                }
                *stage.lock().unwrap() = format!("Exporting to {}", opts.dest.display());
                let ex = export::export(&root, lock.as_ref(), &store, &opts, progress)?;
                let mut out = Outcome { warnings: ex.warnings.clone(), ..Default::default() };
                let mut note = format!("Exported {} files{} to {}", ex.files, if ex.vendored > 0 { format!(" and {} saved from other servers", ex.vendored) } else { String::new() }, ex.path);
                if !push.is_empty() {
                    *stage.lock().unwrap() = format!("Pushing to {push}");
                    let host = url::Url::parse(push).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
                    let token = self.tokens().get(&host).cloned().unwrap_or_default();
                    export::push(&opts.dest, push, &token)?;
                    note.push_str(&format!(", and pushed to {push}"));
                }
                out.note = Some(note);
                Ok(out)
            }
        }
    }

    /// Add a website as a project of its own: saved from `url`, checked like a repo.
    pub(super) fn run_add_web(&self, url: &str, follow: &crate::library::Follow, mirror_settings: Option<&sites::MirrorSettings>, progress: &Progress, stage: &capture::Stage) -> Result<Outcome> {
        let start = mirror::normalise_start(url)?;
        let sref = crate::sources::web_source(&start);
        let id = sref.id();
        let mut repo = Repo::new(sref, follow.clone());
        repo.follow.releases = false;
        repo.site.live_url = start.to_string();
        if let Some(m) = mirror_settings {
            repo.site.mirror = m.clone();
        }
        repo.details.web_url = start.to_string();
        let folder = self.with_lib(|l| {
            if l.repos.contains_key(&id) {
                bail!("{id} is already in the library.");
            }
            Ok(l.folder_of(&id))
        })?;
        std::fs::create_dir_all(folder.join(crate::library::PARTIAL_DIR))?;
        let src = crate::sources::open(&repo.source, &self.tokens(), self.client.clone());
        let limits = self.limits(&repo.follow);
        let res = capture::update(&mut repo, &folder, src.as_ref(), &self.client, progress, stage, &limits);
        if res.is_err() {
            // nothing kept: a wrong address leaves no project behind
            let _ = std::fs::remove_dir_all(&folder);
            return res;
        }
        self.with_lib(|l| {
            l.save_repo(&repo)?;
            l.repos.insert(id.clone(), repo.clone());
            Ok(())
        })?;
        let mut out = res?;
        self.after_capture(&id, &mut out, progress, stage);
        Ok(out)
    }
}

fn rec_exists(wd: &Path) -> bool {
    wd.join("build.log").is_file()
}

/// The `site` part of a project's summary: for the badge on its card.
pub fn badge(app: &App, l: &Library, r: &Repo) -> Value {
    if !app.is_site(l, r) {
        return Value::Null;
    }
    let kind = if r.source.kind == Kind::Web { SiteKind::Mirror } else { r.latest_snapshot().map(|s| app.info_for(l, r, Coll::Code, s).kind).unwrap_or(SiteKind::None) };
    json!({ "kind": kind, "kind_label": kind.label(), "label": label_of(r) })
}
