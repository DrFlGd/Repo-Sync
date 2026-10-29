//! The app's commands: one table, [`App::call`], used by the desktop app
//! (desktop/src-tauri) and by `reposync-cli serve` (the tests). Downloads run as
//! jobs, one at a time, on a worker thread; a second thread queues the checks
//! that are due (docs/PLAN.md, "Updates").

use crate::capture::{self, Limits, Outcome};
use crate::config::{AppConfig, Prefs};
use crate::http::{Client, Progress};
use crate::launch;
use crate::library::{Follow, FollowedAccount, Library, Repo, Settings};
use crate::readme;
use crate::sources::{self, Account, Kind};
use crate::util::{now_iso, now_unix, percent_encode, unix_from_iso};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};

pub struct AppPaths {
    pub config_dir: PathBuf,
    /// The library folder's files as the page reaches them ("library://localhost/").
    pub library_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JobKind {
    /// Add a project from a link, then capture it.
    Add { input: String, follow: Follow },
    /// Check a project and capture what's new.
    Check {
        #[serde(rename = "project")]
        id: String,
    },
    /// List an account's projects and queue the ones not in the library yet;
    /// `watch` follows the account so its new projects are added at each check.
    Account { account: Account, follow: Follow, forks: bool, watch: bool },
    /// Download one release the user picked (Releases, "Keep").
    Release {
        #[serde(rename = "project")]
        id: String,
        tag: String,
    },
}

impl JobKind {
    /// Whether `other` does the same work, so it isn't queued twice. A release
    /// picked by hand sits beside a check of the same project.
    fn same(&self, other: &JobKind) -> bool {
        match (self, other) {
            (JobKind::Release { tag: a, .. }, JobKind::Release { tag: b, .. }) => a == b,
            (JobKind::Release { .. }, _) | (_, JobKind::Release { .. }) => false,
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: u64,
    #[serde(flatten)]
    pub kind: JobKind,
    /// The project, once known.
    pub repo: String,
    /// "you" or "schedule".
    pub by: &'static str,
    pub queued: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finished {
    pub job: Job,
    pub finished: String,
    pub ok: bool,
    pub message: String,
    pub outcome: Option<Outcome>,
}

struct Running {
    job: Job,
    started: String,
    progress: Arc<Progress>,
    stage: Arc<capture::Stage>,
}

#[derive(Default)]
struct Jobs {
    next: u64,
    queue: VecDeque<Job>,
    current: Option<Running>,
    /// Newest first, at most 50.
    recent: VecDeque<Finished>,
}

pub struct App {
    paths: AppPaths,
    config: Mutex<AppConfig>,
    prefs: Prefs,
    lib: Mutex<Option<Library>>,
    lib_error: Mutex<Option<String>>,
    jobs: Mutex<Jobs>,
    wake: Condvar,
    client: Arc<Client>,
    /// Readme text by project, for search: (snapshot folder, lower-case text).
    readmes: Mutex<HashMap<String, (String, String)>>,
}

impl App {
    /// Open the library from this computer's config and start the worker and the
    /// schedule. `tick` is how often the schedule looks for checks that are due.
    pub fn new(paths: AppPaths, tick: std::time::Duration) -> Result<Arc<Self>> {
        let cfg_file = paths.config_dir.join("config.json");
        let config = AppConfig::load(&cfg_file);
        let prefs = Prefs::new(paths.config_dir.join("prefs.json"));
        let app = Arc::new(Self {
            paths,
            config: Mutex::new(config),
            prefs,
            lib: Mutex::new(None),
            lib_error: Mutex::new(None),
            jobs: Mutex::new(Jobs::default()),
            wake: Condvar::new(),
            client: Arc::new(Client::new()),
            readmes: Mutex::new(HashMap::new()),
        });
        let path = app.config.lock().unwrap().library_path();
        app.open_library(path);
        let worker = app.clone();
        std::thread::Builder::new().name("jobs".into()).spawn(move || worker.work())?;
        let sched = app.clone();
        std::thread::Builder::new().name("schedule".into()).spawn(move || loop {
            std::thread::sleep(tick);
            sched.queue_due();
        })?;
        Ok(app)
    }

    fn open_library(&self, path: PathBuf) {
        match Library::open(&path) {
            Ok(l) => {
                *self.lib.lock().unwrap() = Some(l);
                *self.lib_error.lock().unwrap() = None;
                self.readmes.lock().unwrap().clear();
                let mut c = self.config.lock().unwrap();
                c.set_library(&path);
                let _ = c.save(&self.paths.config_dir.join("config.json"));
            }
            Err(e) => {
                *self.lib.lock().unwrap() = None;
                *self.lib_error.lock().unwrap() = Some(format!("{e:#}"));
            }
        }
    }

    fn with_lib<T>(&self, f: impl FnOnce(&mut Library) -> Result<T>) -> Result<T> {
        let mut g = self.lib.lock().unwrap();
        let lib = g.as_mut().ok_or_else(|| anyhow!(self.lib_error.lock().unwrap().clone().unwrap_or_else(|| "No library is open.".into())))?;
        f(lib)
    }

    /// One command, by name, with its arguments as JSON.
    pub fn call(&self, cmd: &str, args: Value) -> Result<Value, String> {
        self.dispatch(cmd, &args).map_err(|e| format!("{e:#}"))
    }

    fn dispatch(&self, cmd: &str, a: &Value) -> Result<Value> {
        let id = || a["id"].as_str().map(String::from).context("no project given");
        match cmd {
            "app_info" => Ok(self.app_info()),
            "library_open" => {
                let p = a["path"].as_str().context("no folder given")?;
                if self.jobs.lock().unwrap().current.is_some() {
                    bail!("Wait for the download in hand to finish (or stop it) before changing library.");
                }
                self.jobs.lock().unwrap().queue.clear();
                self.open_library(PathBuf::from(p));
                if let Some(e) = self.lib_error.lock().unwrap().clone() {
                    bail!(e);
                }
                Ok(self.app_info())
            }
            "prefs_get" => Ok(self.prefs.get()),
            "prefs_set" => {
                self.prefs.set(&a["prefs"])?;
                Ok(json!(true))
            }
            "settings_get" => self.settings_get(),
            "settings_set" => self.settings_set(a),
            "repos" => self.repos(a["query"].as_str().unwrap_or("")),
            "repo" => self.repo(&id()?),
            "readme" => self.readme(&id()?, a["snapshot"].as_str()),
            "release_notes" => self.release_notes(&id()?, a["folder"].as_str().context("no release given")?),
            "link_check" => self.link_check(a["input"].as_str().unwrap_or("")),
            "add" => self.add(a),
            "check" => {
                let id = id()?;
                self.with_lib(|l| l.repos.get(&id).map(|_| ()).context("That project isn't in the library."))?;
                Ok(json!({ "job": self.enqueue(JobKind::Check { id: id.clone() }, id, "you") }))
            }
            "check_all" => {
                let ids: Vec<String> = self.with_lib(|l| Ok(l.repos.keys().cloned().collect()))?;
                let mut n = ids.iter().filter_map(|id| self.enqueue(JobKind::Check { id: id.clone() }, id.clone(), "you")).count();
                n += self.accounts_due(true).into_iter().filter_map(|a| self.enqueue_account(a, "you")).count();
                Ok(json!({ "queued": n }))
            }
            "follow_set" => {
                let id = id()?;
                let f: Follow = serde_json::from_value(a["follow"].clone()).context("those settings can't be read")?;
                self.edit(&id, |r| {
                    r.follow = f;
                    Ok(())
                })
            }
            "tags_set" => {
                let id = id()?;
                let mut tags: Vec<String> = a["tags"].as_array().map(|t| t.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
                tags.dedup();
                self.edit(&id, |r| {
                    r.tags = tags;
                    Ok(())
                })
            }
            "notes_set" => {
                let id = id()?;
                let notes = a["notes"].as_str().unwrap_or("").to_string();
                self.edit(&id, |r| {
                    r.notes = notes;
                    Ok(())
                })
            }
            "releases_available" => self.releases_available(&id()?),
            "accounts" => self.with_lib(|l| Ok(json!(l.accounts().iter().map(|a| json!({ "id": a.account.id(), "kind": a.account.kind.label(), "web_url": a.account.web_url(), "forks": a.forks, "added": a.added, "last_checked": a.last_checked })).collect::<Vec<_>>()))),
            "account_remove" => {
                let id = id()?;
                self.with_lib(|l| {
                    let mut all = l.accounts();
                    all.retain(|a| a.account.id() != id);
                    l.save_accounts(&all)?;
                    Ok(json!(true))
                })
            }
            "release_get" => {
                let id = id()?;
                let tag = a["tag"].as_str().filter(|t| !t.is_empty()).context("no release given")?.to_string();
                self.with_lib(|l| l.repos.get(&id).map(|_| ()).context("That project isn't in the library."))?;
                Ok(json!({ "job": self.enqueue(JobKind::Release { id: id.clone(), tag }, id, "you") }))
            }
            "branches" => {
                let id = id()?;
                let source = self.with_lib(|l| Ok(l.repos.get(&id).context("That project isn't in the library.")?.source.clone()))?;
                Ok(json!(sources::open(&source, &self.tokens(), self.client.clone()).branches()?))
            }
            "remove" => self.remove(&id()?),
            "version_delete" => self.version_delete(&id()?, a["kind"].as_str().unwrap_or(""), a["folder"].as_str().unwrap_or("")),
            "launch_list" => self.launch_list(&id()?, a["folder"].as_str()),
            "launch" => self.launch(&id()?, a["folder"].as_str(), a["file"].as_str().unwrap_or(""), a["remember"].as_bool().unwrap_or(false)),
            "jobs" => Ok(self.jobs_json()),
            "job_stop" => {
                let job = a["job"].as_u64();
                let mut j = self.jobs.lock().unwrap();
                if let Some(c) = &j.current {
                    if job.is_none() || job == Some(c.job.id) {
                        c.progress.stop.store(true, Ordering::Relaxed);
                    }
                }
                if let Some(job) = job {
                    j.queue.retain(|q| q.id != job);
                } else {
                    j.queue.clear();
                }
                Ok(json!(true))
            }
            "path" => {
                let id = id()?;
                let sub = a["sub"].as_str().unwrap_or("");
                self.with_lib(|l| {
                    let base = l.folder_of(&id);
                    let p = if sub.is_empty() { base } else { base.join(crate::util::safe_rel(std::path::Path::new(sub)).context("not a folder of the project")?) };
                    Ok(json!(p.display().to_string()))
                })
            }
            other => bail!("unknown command {other}"),
        }
    }

    fn app_info(&self) -> Value {
        let lib = self.lib.lock().unwrap();
        let cfg = self.config.lock().unwrap();
        json!({
            "version": crate::VERSION,
            "os": std::env::consts::OS,
            "library_url": self.paths.library_url,
            "library": lib.as_ref().map(|l| json!({ "path": l.root.display().to_string(), "name": l.name, "id": l.id, "repos": l.repos.len() })),
            "library_error": *self.lib_error.lock().unwrap(),
            "recent_libraries": cfg.recent_libraries.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "default_library": crate::config::default_library_path().display().to_string(),
        })
    }

    fn settings_get(&self) -> Result<Value> {
        let tokens: Value = self.config.lock().unwrap().tokens.iter().map(|(h, t)| (h.clone(), json!(!t.is_empty()))).collect::<serde_json::Map<_, _>>().into();
        self.with_lib(|l| Ok(json!({ "library": l.settings, "tokens": tokens, "next_check": self.next_check(l) })))
    }

    fn settings_set(&self, a: &Value) -> Result<Value> {
        if let Some(t) = a["tokens"].as_object() {
            let mut c = self.config.lock().unwrap();
            for (host, v) in t {
                match v.as_str().map(str::trim) {
                    Some("") | None => {
                        c.tokens.remove(host);
                    }
                    Some(tok) => {
                        c.tokens.insert(host.to_ascii_lowercase(), tok.to_string());
                    }
                }
            }
            c.save(&self.paths.config_dir.join("config.json"))?;
        }
        if !a["library"].is_null() {
            let s: Settings = serde_json::from_value(a["library"].clone()).context("those settings can't be read")?;
            self.with_lib(|l| l.save_settings(s))?;
        }
        self.settings_get()
    }

    /// When the schedule will next check something (ISO 8601), or null.
    fn next_check(&self, l: &Library) -> Value {
        l.repos.values().filter_map(|r| due_at(r, &l.settings)).min().map(|t| json!(crate::util::iso_from_unix(t.max(now_unix())))).unwrap_or(Value::Null)
    }

    fn status_of(&self, id: &str) -> &'static str {
        let j = self.jobs.lock().unwrap();
        if j.current.as_ref().is_some_and(|c| c.job.repo == id) {
            "working"
        } else if j.queue.iter().any(|q| q.repo == id) {
            "queued"
        } else {
            ""
        }
    }

    fn summary(&self, l: &Library, r: &Repo) -> Value {
        let id = r.id();
        let status = self.status_of(&id);
        let launchable = r.releases.iter().rev().find(|x| x.assets.iter().any(|a| !a.file.is_empty()));
        json!({
            "id": id,
            "kind": r.source.kind,
            "host": r.source.host,
            "owner": r.source.owner(),
            "name": if r.details.name.is_empty() { r.source.name().to_string() } else { r.details.name.clone() },
            "description": r.details.description,
            "language": r.details.language,
            "topics": r.details.topics,
            "tags": r.tags,
            "stars": r.details.stars,
            "archived": r.details.archived,
            "license": r.details.license,
            "pushed_at": r.details.pushed_at,
            "added": r.added,
            "last_checked": r.last_checked,
            "last_changed": r.last_changed,
            "error": r.last_error,
            "status": if !status.is_empty() { status } else if !r.last_error.is_empty() { "error" } else { "ok" },
            "bytes": r.bytes(),
            "snapshots": r.snapshots.len(),
            "releases": r.releases.len(),
            "latest_snapshot": r.latest_snapshot().map(|s| json!({ "folder": s.folder, "commit": s.commit, "captured": s.captured, "commit_date": s.commit_date })),
            "latest_release": r.latest_release().map(|x| json!({ "tag": x.tag, "name": x.name, "published": x.published, "folder": x.folder })),
            "launchable": launchable.map(|x| x.folder.clone()),
            "follow": r.follow,
            "due": due_at(r, &l.settings).map(crate::util::iso_from_unix),
        })
    }

    fn readme_text(&self, l: &Library, r: &Repo) -> String {
        let Some(s) = r.latest_snapshot() else { return String::new() };
        if s.readme.is_empty() {
            return String::new();
        }
        let id = r.id();
        if let Some((folder, text)) = self.readmes.lock().unwrap().get(&id) {
            if *folder == s.folder {
                return text.clone();
            }
        }
        let p = l.folder_of(&id).join("source").join(&s.folder).join(&s.readme);
        let mut text = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).to_lowercase()).unwrap_or_default();
        text.truncate(text.char_indices().nth(200_000).map(|(i, _)| i).unwrap_or(text.len()));
        self.readmes.lock().unwrap().insert(id, (s.folder.clone(), text.clone()));
        text
    }

    fn repos(&self, query: &str) -> Result<Value> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
        self.with_lib(|l| {
            let mut out = vec![];
            for r in l.repos.values() {
                let mut v = self.summary(l, r);
                if !words.is_empty() {
                    let meta = format!("{} {} {} {} {} {}", r.id(), r.details.description, r.details.language, r.details.topics.join(" "), r.tags.join(" "), r.notes).to_lowercase();
                    if !words.iter().all(|w| meta.contains(w.as_str())) {
                        let text = self.readme_text(l, r);
                        if !words.iter().all(|w| meta.contains(w.as_str()) || text.contains(w.as_str())) {
                            continue;
                        }
                        v["match"] = json!("readme");
                    }
                }
                out.push(v);
            }
            let total: u64 = l.repos.values().map(Repo::bytes).sum();
            Ok(json!({
                "repos": out,
                "count": l.repos.len(),
                "bytes": total,
                "unreadable": l.unreadable.iter().map(|(f, e)| json!({ "folder": f, "error": e })).collect::<Vec<_>>(),
                "next_check": self.next_check(l),
            }))
        })
    }

    fn repo(&self, id: &str) -> Result<Value> {
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let mut v = serde_json::to_value(r)?;
            v["summary"] = self.summary(l, r);
            v["path"] = json!(l.folder_of(id).display().to_string());
            v["web_url"] = json!(if r.details.web_url.is_empty() { r.source.web_url() } else { r.details.web_url.clone() });
            Ok(v)
        })
    }

    fn file_base(&self, id: &str) -> String {
        let enc: Vec<String> = id.split('/').map(percent_encode).collect();
        format!("{}{}/", self.paths.library_url, enc.join("/"))
    }

    fn readme(&self, id: &str, snapshot: Option<&str>) -> Result<Value> {
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let s = match snapshot {
                Some(f) => r.snapshots.iter().find(|s| s.folder == f).context("That version isn't in the library.")?,
                None => match r.latest_snapshot() {
                    Some(s) => s,
                    None => return Ok(json!({ "html": null, "file": null })),
                },
            };
            if s.readme.is_empty() {
                return Ok(json!({ "html": null, "file": null, "snapshot": s.folder }));
            }
            let p = l.folder_of(id).join("source").join(&s.folder).join(&s.readme);
            let bytes = std::fs::read(&p).with_context(|| format!("couldn't read {}", p.display()))?;
            let web_root = if r.details.web_url.is_empty() { r.source.web_url() } else { r.details.web_url.clone() };
            let web = match r.source.kind {
                Kind::GitHub => format!("{web_root}/blob/{}/", s.commit),
                Kind::GitLab => format!("{web_root}/-/blob/{}/", s.commit),
                Kind::Gitea => format!("{web_root}/src/commit/{}/", s.commit),
            };
            let dir = s.readme.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
            let bases = readme::Bases { files: format!("{}source/{}/", self.file_base(id), percent_encode(&s.folder)), web, dir };
            Ok(json!({ "html": readme::to_html(&s.readme, &bytes, &bases), "file": s.readme, "snapshot": s.folder }))
        })
    }

    fn release_notes(&self, id: &str, folder: &str) -> Result<Value> {
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let rel = r.releases.iter().find(|x| x.folder == folder).context("That release isn't in the library.")?;
            let p = l.folder_of(id).join("releases").join(&rel.folder).join("RELEASE_NOTES.md");
            let bytes = std::fs::read(&p).unwrap_or_default();
            let bases = readme::Bases { files: format!("{}releases/{}/", self.file_base(id), percent_encode(&rel.folder)), web: String::new(), dir: String::new() };
            Ok(json!({ "html": readme::to_html("RELEASE_NOTES.md", &bytes, &bases) }))
        })
    }

    /// What a typed link names, without asking the server (for the Add dialog).
    fn link_check(&self, input: &str) -> Result<Value> {
        let mut out = vec![];
        for line in input.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if sources::parse_url(line).is_err() {
                if let Some(acc) = sources::parse_account(line) {
                    let following = self.with_lib(|l| Ok(l.accounts().iter().any(|a| a.account == acc))).unwrap_or(false);
                    out.push(json!({ "input": line, "ok": true, "account": { "id": acc.id(), "name": acc.name, "host": acc.host, "kind": acc.kind.label() }, "following": following }));
                    continue;
                }
            }
            out.push(match sources::parse_url(line) {
                Ok(p) => {
                    let id = format!("{}/{}", p.host, p.path);
                    let have = self.with_lib(|l| Ok(l.repos.contains_key(&id))).unwrap_or(false);
                    json!({ "input": line, "ok": true, "id": id, "host": p.host, "path": p.path, "kind": p.kind.map(|k| k.label()), "in_library": have })
                }
                Err(e) => json!({ "input": line, "ok": false, "error": e.to_string() }),
            });
        }
        Ok(json!(out))
    }

    fn add(&self, a: &Value) -> Result<Value> {
        let inputs: Vec<String> = match &a["inputs"] {
            Value::Array(v) => v.iter().filter_map(|x| x.as_str()).map(String::from).collect(),
            _ => a["input"].as_str().unwrap_or("").lines().map(String::from).collect(),
        };
        let inputs: Vec<String> = inputs.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if inputs.is_empty() {
            bail!("Paste a link to a repository, such as https://github.com/owner/name.");
        }
        let follow: Follow = if a["follow"].is_object() {
            serde_json::from_value(a["follow"].clone()).context("those settings can't be read")?
        } else {
            self.with_lib(|l| Ok(l.settings.new_follow.clone()))?
        };
        let forks = a["forks"].as_bool().unwrap_or(false);
        let watch = a["watch"].as_bool().unwrap_or(true);
        let mut out = vec![];
        for input in inputs {
            if let (Err(_), Some(acc)) = (sources::parse_url(&input), sources::parse_account(&input)) {
                let key = format!("account:{}", acc.id());
                let id = acc.id();
                let job = self.enqueue(JobKind::Account { account: acc, follow: follow.clone(), forks, watch }, key, "you");
                out.push(json!({ "input": input, "account": id, "job": job }));
                continue;
            }
            match sources::parse_url(&input) {
                Err(e) => out.push(json!({ "input": input, "error": e.to_string() })),
                Ok(p) => {
                    let id = format!("{}/{}", p.host, p.path);
                    if self.with_lib(|l| Ok(l.repos.contains_key(&id)))? {
                        out.push(json!({ "input": input, "id": id, "error": "already in the library" }));
                        continue;
                    }
                    let job = self.enqueue(JobKind::Add { input: input.clone(), follow: follow.clone() }, id.clone(), "you");
                    out.push(json!({ "input": input, "id": id, "job": job }));
                }
            }
        }
        Ok(json!(out))
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut Repo) -> Result<()>) -> Result<Value> {
        self.with_lib(|l| {
            let mut r = l.repos.get(id).cloned().context("That project isn't in the library.")?;
            f(&mut r)?;
            l.save_repo(&r)?;
            l.repos.insert(id.to_string(), r);
            Ok(json!(true))
        })?;
        self.repo(id)
    }

    fn remove(&self, id: &str) -> Result<Value> {
        if self.status_of(id) == "working" {
            bail!("That project is downloading: stop it first.");
        }
        self.jobs.lock().unwrap().queue.retain(|q| q.repo != id);
        self.with_lib(|l| {
            let r = l.repos.remove(id).context("That project isn't in the library.")?;
            // a followed account's project removed by hand isn't added back by the next check
            let mut accounts = l.accounts();
            let owner = format!("{}/{}", r.source.host, r.source.owner());
            let mut changed = false;
            for a in accounts.iter_mut().filter(|a| a.account.id().eq_ignore_ascii_case(&owner) && !a.removed.iter().any(|x| x == id)) {
                a.removed.push(id.to_string());
                changed = true;
            }
            if changed {
                l.save_accounts(&accounts)?;
            }
            let dir = l.folder_of(id);
            std::fs::remove_dir_all(&dir).with_context(|| format!("couldn't remove {}", dir.display()))?;
            // tidy the owner's and host's folders when they're empty now
            let mut p = dir.parent().map(PathBuf::from);
            while let Some(d) = p {
                if d == l.root || std::fs::remove_dir(&d).is_err() {
                    break;
                }
                p = d.parent().map(PathBuf::from);
            }
            Ok(json!(true))
        })
    }

    fn version_delete(&self, id: &str, kind: &str, folder: &str) -> Result<Value> {
        if self.status_of(id) == "working" {
            bail!("That project is downloading: wait for it to finish.");
        }
        self.with_lib(|l| {
            let mut r = l.repos.get(id).cloned().context("That project isn't in the library.")?;
            let sub = match kind {
                "source" => {
                    let n = r.snapshots.len();
                    r.snapshots.retain(|s| s.folder != folder);
                    if r.snapshots.len() == n {
                        bail!("That version isn't in the library.");
                    }
                    "source"
                }
                "release" => {
                    let n = r.releases.len();
                    r.releases.retain(|s| s.folder != folder);
                    if r.releases.len() == n {
                        bail!("That release isn't in the library.");
                    }
                    "releases"
                }
                _ => bail!("unknown kind of version"),
            };
            let dir = l.folder_of(id).join(sub).join(crate::util::safe_rel(std::path::Path::new(folder)).context("not a version's folder")?);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).with_context(|| format!("couldn't remove {}", dir.display()))?;
            }
            l.save_repo(&r)?;
            l.repos.insert(id.to_string(), r);
            Ok(())
        })?;
        self.repo(id)
    }

    /// The release to launch from: `folder`, or the newest one with files.
    fn release_dir(&self, id: &str, folder: Option<&str>) -> Result<(PathBuf, String, String, String)> {
        self.with_lib(|l| {
            let r = l.repos.get(id).context("That project isn't in the library.")?;
            let rel = match folder {
                Some(f) => r.releases.iter().find(|x| x.folder == f).context("That release isn't in the library.")?,
                None => r.releases.iter().rev().find(|x| x.assets.iter().any(|a| !a.file.is_empty())).context("No release with files has been captured yet.")?,
            };
            Ok((l.folder_of(id).join("releases").join(&rel.folder), rel.folder.clone(), r.source.name().to_string(), r.launch.file.clone()))
        })
    }

    fn launch_list(&self, id: &str, folder: Option<&str>) -> Result<Value> {
        let (dir, folder, name, remembered) = self.release_dir(id, folder)?;
        let c = launch::candidates(&dir, &name);
        let chosen = if !remembered.is_empty() && dir.join(&remembered).is_file() { remembered } else { c.first().map(|x| x.file.clone()).unwrap_or_default() };
        Ok(json!({ "folder": folder, "candidates": c, "chosen": chosen }))
    }

    fn launch(&self, id: &str, folder: Option<&str>, file: &str, remember: bool) -> Result<Value> {
        let (dir, folder, name, remembered) = self.release_dir(id, folder)?;
        // the file chosen before, when it's in this release too
        let file = if file.is_empty() && !remembered.is_empty() && dir.join(&remembered).is_file() { remembered } else { file.to_string() };
        let started = launch::launch(&dir, &file, &name)?;
        if remember && !file.is_empty() {
            let f = file.clone();
            self.edit(id, |r| {
                r.launch.file = f;
                Ok(())
            })?;
        }
        Ok(json!({ "folder": folder, "file": started.file, "pid": started.pid }))
    }

    // ------------------------------------------------------------------ jobs

    /// Queue a job, unless the same one is queued or running. Its id, or None.
    fn enqueue(&self, kind: JobKind, repo: String, by: &'static str) -> Option<u64> {
        let mut j = self.jobs.lock().unwrap();
        if j.queue.iter().any(|q| q.repo == repo && q.kind.same(&kind)) || j.current.as_ref().is_some_and(|c| c.job.repo == repo && c.job.kind.same(&kind)) {
            return None;
        }
        j.next += 1;
        let id = j.next;
        j.queue.push_back(Job { id, kind, repo, by, queued: now_iso() });
        self.wake.notify_all();
        Some(id)
    }

    /// The followed accounts due a look for new projects (every one with `all`).
    fn accounts_due(&self, all: bool) -> Vec<FollowedAccount> {
        let now = now_unix();
        match self.lib.lock().unwrap().as_ref() {
            Some(l) => {
                let hours = l.settings.check_every_hours;
                l.accounts()
                    .into_iter()
                    .filter(|a| all || (hours > 0.0 && unix_from_iso(&a.last_checked).unwrap_or(0) + (hours * 3600.0) as i64 <= now))
                    .collect()
            }
            None => vec![],
        }
    }

    fn enqueue_account(&self, a: FollowedAccount, by: &'static str) -> Option<u64> {
        let key = format!("account:{}", a.account.id());
        self.enqueue(JobKind::Account { account: a.account, follow: a.follow, forks: a.forks, watch: true }, key, by)
    }

    fn queue_due(&self) {
        let now = now_unix();
        let due: Vec<String> = match self.lib.lock().unwrap().as_ref() {
            Some(l) => l.repos.values().filter(|r| due_at(r, &l.settings).is_some_and(|t| t <= now)).map(Repo::id).collect(),
            None => return,
        };
        for id in due {
            self.enqueue(JobKind::Check { id: id.clone() }, id, "schedule");
        }
        for a in self.accounts_due(false) {
            self.enqueue_account(a, "schedule");
        }
    }

    fn jobs_json(&self) -> Value {
        let j = self.jobs.lock().unwrap();
        json!({
            "current": j.current.as_ref().map(|c| json!({
                "job": c.job,
                "started": c.started,
                "stage": *c.stage.lock().unwrap(),
                "done": c.progress.done.load(Ordering::Relaxed),
                "total": c.progress.total.load(Ordering::Relaxed),
            })),
            "queued": j.queue,
            "recent": j.recent,
        })
    }

    fn work(self: Arc<Self>) {
        loop {
            let (job, progress, stage) = {
                let mut j = self.jobs.lock().unwrap();
                while j.queue.is_empty() {
                    j = self.wake.wait(j).unwrap();
                }
                let job = j.queue.pop_front().unwrap();
                let progress = Arc::new(Progress::default());
                let stage = Arc::new(Mutex::new(String::from("Starting")));
                j.current = Some(Running { job: job.clone(), started: now_iso(), progress: progress.clone(), stage: stage.clone() });
                (job, progress, stage)
            };
            let res = match &job.kind {
                JobKind::Add { input, follow } => self.run_add(input, follow, &progress, &stage),
                JobKind::Check { id } => self.run_check(id, &progress, &stage),
                JobKind::Release { id, tag } => self.run_release(id, tag, &progress, &stage),
                JobKind::Account { account, follow, forks, watch } => self.run_account(account, follow, *forks, *watch, &stage),
            };
            let mut j = self.jobs.lock().unwrap();
            j.current = None;
            let (ok, message, outcome) = match res {
                Ok(o) => (o.warnings.is_empty(), describe(&o), Some(o)),
                Err(e) => (false, format!("{e:#}"), None),
            };
            j.recent.push_front(Finished { job, finished: now_iso(), ok, message, outcome });
            j.recent.truncate(50);
        }
    }

    fn tokens(&self) -> sources::Tokens {
        self.config.lock().unwrap().tokens.clone()
    }

    fn limits(&self) -> Limits {
        let mb = self.lib.lock().unwrap().as_ref().map(|l| l.settings.max_file_mb).unwrap_or(0);
        Limits { max_file: mb << 20 }
    }

    fn run_add(&self, input: &str, follow: &Follow, progress: &Progress, stage: &capture::Stage) -> Result<Outcome> {
        let parsed = sources::parse_url(input)?;
        *stage.lock().unwrap() = format!("Finding {}", parsed.path);
        let sref = sources::probe(&parsed, &self.tokens(), self.client.clone())?;
        let src = sources::open(&sref, &self.tokens(), self.client.clone());
        // ask for the details first, so a wrong link leaves nothing behind
        let details = src.details()?;
        let id = sref.id();
        let mut repo = Repo::new(sref, follow.clone());
        repo.details = details;
        let folder = self.with_lib(|l| {
            if l.repos.contains_key(&id) {
                bail!("{id} is already in the library.");
            }
            l.save_repo(&repo)?;
            l.repos.insert(id.clone(), repo.clone());
            Ok(l.folder_of(&id))
        })?;
        let res = capture::update(&mut repo, &folder, src.as_ref(), &self.client, progress, stage, &self.limits());
        self.store(&id, &repo, &res)?;
        res
    }

    fn run_check(&self, id: &str, progress: &Progress, stage: &capture::Stage) -> Result<Outcome> {
        let (mut repo, folder) = self.with_lib(|l| Ok((l.repos.get(id).cloned().context("That project isn't in the library any more.")?, l.folder_of(id))))?;
        let src = sources::open(&repo.source, &self.tokens(), self.client.clone());
        let res = capture::update(&mut repo, &folder, src.as_ref(), &self.client, progress, stage, &self.limits());
        self.store(id, &repo, &res)?;
        res
    }

    fn run_release(&self, id: &str, tag: &str, progress: &Progress, stage: &capture::Stage) -> Result<Outcome> {
        let (mut repo, folder) = self.with_lib(|l| Ok((l.repos.get(id).cloned().context("That project isn't in the library any more.")?, l.folder_of(id))))?;
        let src = sources::open(&repo.source, &self.tokens(), self.client.clone());
        let res = capture::capture_tag(&mut repo, &folder, tag, src.as_ref(), &self.client, progress, stage, &self.limits());
        if res.is_ok() {
            self.with_lib(|l| {
                let Some(cur) = l.repos.get(id) else { return Ok(()) };
                let mut r = cur.clone();
                r.releases = repo.releases.clone();
                r.last_changed = repo.last_changed.clone();
                l.save_repo(&r)?;
                l.repos.insert(id.to_string(), r);
                Ok(())
            })?;
        }
        res
    }

    fn run_account(&self, account: &Account, follow: &Follow, forks: bool, watch: bool, stage: &capture::Stage) -> Result<Outcome> {
        *stage.lock().unwrap() = format!("Listing the projects of {}", account.name);
        let list = sources::projects(account, &self.tokens(), &self.client)?;
        let (have, removed) = self.with_lib(|l| {
            let removed = l.accounts().into_iter().find(|a| a.account == *account).map(|a| a.removed).unwrap_or_default();
            Ok((l.repos.keys().cloned().collect::<Vec<_>>(), removed))
        })?;
        let mut queued = 0;
        let mut skipped_forks = 0;
        for p in &list {
            let id = format!("{}/{}", account.host, p.path);
            if have.contains(&id) || removed.contains(&id) {
                continue;
            }
            if p.fork && !forks {
                skipped_forks += 1;
                continue;
            }
            let input = format!("https://{id}");
            if self.enqueue(JobKind::Add { input, follow: follow.clone() }, id, "you").is_some() {
                queued += 1;
            }
        }
        if watch {
            self.with_lib(|l| {
                let mut all = l.accounts();
                match all.iter_mut().find(|a| a.account == *account) {
                    Some(a) => {
                        a.last_checked = now_iso();
                        a.follow = follow.clone();
                        a.forks = forks;
                    }
                    None => all.push(FollowedAccount { account: account.clone(), follow: follow.clone(), forks, added: now_iso(), last_checked: now_iso(), removed: vec![] }),
                }
                l.save_accounts(&all)
            })?;
        }
        let mut note = format!("{} has {}", account.id(), plural(list.len(), "project"));
        note.push_str(&match queued {
            0 => ": nothing new to add".to_string(),
            n => format!(": {} to add", plural(n, "new one")),
        });
        if skipped_forks > 0 {
            note.push_str(&format!(" ({} left out)", plural(skipped_forks, "fork")));
        }
        Ok(Outcome { note: Some(note), ..Default::default() })
    }

    /// Every release the server lists for a project, newest first, with which are
    /// kept and which are waiting to be downloaded.
    fn releases_available(&self, id: &str) -> Result<Value> {
        let (repo, folder) = self.with_lib(|l| Ok((l.repos.get(id).cloned().context("That project isn't in the library.")?, l.folder_of(id))))?;
        let src = sources::open(&repo.source, &self.tokens(), self.client.clone());
        let list = src.releases(capture::RELEASES_LISTED)?;
        let waiting: Vec<String> = {
            let j = self.jobs.lock().unwrap();
            j.queue.iter().chain(j.current.as_ref().map(|c| &c.job)).filter(|q| q.repo == id).filter_map(|q| match &q.kind {
                JobKind::Release { tag, .. } => Some(tag.clone()),
                _ => None,
            }).collect()
        };
        let out: Vec<Value> = list
            .iter()
            .map(|r| {
                let kept = repo.releases.iter().rev().find(|k| k.tag == r.tag && folder.join("releases").join(&k.folder).is_dir());
                json!({
                    "tag": r.tag,
                    "name": r.name,
                    "published": r.published,
                    "prerelease": r.prerelease,
                    "web_url": r.web_url,
                    "files": r.assets.len(),
                    "bytes": r.assets.iter().map(|a| a.size).sum::<u64>(),
                    "kept": kept.map(|k| k.folder.clone()),
                    "waiting": waiting.contains(&r.tag),
                })
            })
            .collect();
        Ok(json!(out))
    }

    /// Save what a check found into the project as it is now (its settings may
    /// have changed while it ran).
    fn store(&self, id: &str, checked: &Repo, res: &Result<Outcome>) -> Result<()> {
        self.with_lib(|l| {
            let Some(cur) = l.repos.get(id) else { return Ok(()) };
            let mut r = cur.clone();
            r.details = checked.details.clone();
            r.snapshots = checked.snapshots.clone();
            r.releases = checked.releases.clone();
            match res {
                Ok(_) => {
                    r.last_checked = checked.last_checked.clone();
                    r.last_changed = checked.last_changed.clone();
                    r.last_error = checked.last_error.clone();
                }
                Err(e) => {
                    r.last_checked = now_iso();
                    r.last_error = format!("{e:#}");
                }
            }
            l.save_repo(&r)?;
            l.repos.insert(id.to_string(), r);
            Ok(())
        })
    }
}

/// When a project is next due a check (seconds since 1970), None when it's only checked by hand.
fn due_at(r: &Repo, s: &Settings) -> Option<i64> {
    let hours = r.follow.every_hours.unwrap_or(s.check_every_hours);
    if hours <= 0.0 || !(r.follow.source || r.follow.releases) {
        return None;
    }
    let last = unix_from_iso(&r.last_checked).or_else(|| unix_from_iso(&r.added)).unwrap_or(0);
    Some(last + (hours * 3600.0) as i64)
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn describe(o: &Outcome) -> String {
    if let Some(n) = &o.note {
        return n.clone();
    }
    let mut parts = vec![];
    if let Some(s) = &o.snapshot {
        parts.push(format!("new version of the code ({s})"));
    }
    if let Some(r) = &o.release {
        parts.push(format!("new release {r}"));
    }
    let mut msg = if parts.is_empty() { "Up to date".to_string() } else { format!("Captured: {}", parts.join(", ")) };
    for w in &o.warnings {
        msg.push_str(&format!(". {w}"));
    }
    msg
}

impl App {
    /// A file in the open library, for library:// addresses (pictures in readmes).
    pub fn library_file(&self, rel: &str) -> Result<Vec<u8>> {
        let p = self.with_lib(|l| l.inside(&crate::util::percent_decode(rel.trim_start_matches('/'))))?;
        std::fs::read(&p).with_context(|| format!("{} isn't there", p.display()))
    }
}
