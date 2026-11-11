//! The web server that hosts sites on this computer: it listens only on
//! 127.0.0.1 and ::1, gives every site its own address and origin
//! (`http://<name>.<owner>.localhost:<port>/<base>/`, and one per kept version),
//! answers only requests for those names, and serves files the way GitHub Pages
//! does ([`super::router`]), with the addresses of saved files from other
//! servers swapped in ([`super::rewrite`]). It has no commands: nothing a site
//! does reaches the app (docs/PLAN.md, "Serving model", "Security").

use super::rewrite::{self, ROUTE};
use super::router::{self, Decision, Hide, Redirect};
use super::vendor::{self, Allow, Lock, Store};
use super::mime;
use crate::http::Client;
use crate::util::now_iso;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tiny_http::{Header, Request, Response, StatusCode};

/// What the server needs to serve one site version.
#[derive(Clone)]
pub struct View {
    /// The project's id, for the request log.
    pub project: String,
    pub title: String,
    /// The folder the site is published from.
    pub root: PathBuf,
    pub base: String,
    pub hide: Hide,
    pub spa: bool,
    pub markdown: bool,
    /// The version's lock file (saved files from other servers), if it has one.
    pub lock: Option<PathBuf>,
    pub store: Store,
    pub allow: Allow,
    pub headers: Vec<(String, String)>,
    /// Pages may not reach the internet (an enforced policy, to test offline use).
    pub block_internet: bool,
    /// Save files the page asks for that the lock lacks (on a kept server), while online.
    pub save_misses: bool,
    /// For a captured website: where missing pages and files are fetched from
    /// while you browse ("https://agilancer.github.io"), None to not fetch.
    pub record_from: Option<String>,
}

/// One hosted site, for the list at the server's own address.
#[derive(Clone, Debug, Serialize)]
pub struct Listed {
    pub label: String,
    pub title: String,
    pub base: String,
}

/// Where the server learns which sites there are.
pub trait Sites: Send + Sync {
    /// The site a host name names (`<label>` of `<label>.localhost`).
    fn view(&self, label: &str) -> Option<View>;
    fn list(&self) -> Vec<Listed>;
    fn client(&self) -> Arc<Client>;
}

/// One request, for a site's Recent requests.
#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    pub time: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    listeners: Vec<Arc<tiny_http::Server>>,
    log: Mutex<HashMap<String, VecDeque<LogEntry>>>,
    locks: Mutex<HashMap<PathBuf, (SystemTime, Arc<Lock>)>>,
    rules: Mutex<HashMap<PathBuf, (SystemTime, Arc<(Vec<Redirect>, Vec<router::HeaderRule>)>)>>,
    /// One fetch of a missing file at a time.
    fetching: Mutex<()>,
    sites: Arc<dyn Sites>,
}

const WORKERS: usize = 12;

/// Start the server on `port` (or a free port from 20000–40000 when None or taken
/// and `fallback`). Returns once it's listening.
pub fn start(port: Option<u16>, fallback: bool, sites: Arc<dyn Sites>) -> Result<Arc<Server>> {
    let mut tries: Vec<u16> = port.into_iter().collect();
    if fallback || tries.is_empty() {
        let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 ^ d.as_secs()).unwrap_or(7) ^ (std::process::id() as u64) << 7;
        let mut x = seed | 1;
        for _ in 0..40 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            tries.push(20000 + (x % 20000) as u16);
        }
    }
    let mut last_err = String::new();
    for p in tries {
        let v4 = match tiny_http::Server::http(("127.0.0.1", p)) {
            Ok(s) => s,
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        };
        let mut listeners = vec![Arc::new(v4)];
        // ::1 too (browsers may try it first for *.localhost); fine if there's no IPv6
        if let Ok(v6) = tiny_http::Server::http(("::1", p)) {
            listeners.push(Arc::new(v6));
        }
        let server = Arc::new(Server {
            port: p,
            stop: Arc::new(AtomicBool::new(false)),
            listeners,
            log: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            rules: Mutex::new(HashMap::new()),
            fetching: Mutex::new(()),
            sites,
        });
        for l in &server.listeners {
            for i in 0..WORKERS {
                let (l, s) = (l.clone(), server.clone());
                std::thread::Builder::new().name(format!("sites-{p}-{i}")).spawn(move || {
                    while !s.stop.load(Ordering::Relaxed) {
                        match l.recv() {
                            Ok(rq) => s.handle(rq),
                            Err(_) => break,
                        }
                    }
                })?;
            }
        }
        return Ok(server);
    }
    bail!("Couldn't find a free port to host sites on ({last_err}).")
}

impl Server {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        for l in &self.listeners {
            l.unblock();
        }
    }

    /// A site's recent requests, newest first.
    pub fn log_of(&self, project: &str) -> Vec<LogEntry> {
        self.log.lock().unwrap().get(project).map(|q| q.iter().rev().cloned().collect()).unwrap_or_default()
    }

    fn note(&self, project: &str, method: &str, path: &str, status: u16, note: &str) {
        let mut log = self.log.lock().unwrap();
        let q = log.entry(project.to_string()).or_default();
        q.push_back(LogEntry { time: now_iso(), method: method.into(), path: path.chars().take(300).collect(), status, note: note.into() });
        while q.len() > 200 {
            q.pop_front();
        }
    }

    fn lock_of(&self, path: &Path) -> Option<Arc<Lock>> {
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        let mut c = self.locks.lock().unwrap();
        if let Some((t, l)) = c.get(path) {
            if *t == mtime {
                return Some(l.clone());
            }
        }
        let l = Arc::new(vendor::read_lock(path)?);
        c.insert(path.to_path_buf(), (mtime, l.clone()));
        Some(l)
    }

    /// The site's `_redirects` and `_headers` (Netlify's files), and a captured
    /// site's redirects, read again when they change.
    fn rules_of(&self, root: &Path) -> Arc<(Vec<Redirect>, Vec<router::HeaderRule>)> {
        let files = [root.join("_redirects"), root.join("_headers"), root.join("_mirror").join("redirects")];
        let stamp = files.iter().filter_map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).max().unwrap_or(SystemTime::UNIX_EPOCH);
        let mut c = self.rules.lock().unwrap();
        if let Some((t, r)) = c.get(root) {
            if *t == stamp {
                return r.clone();
            }
        }
        let mut redirects = router::parse_redirects(&std::fs::read_to_string(&files[2]).unwrap_or_default());
        redirects.extend(router::parse_redirects(&std::fs::read_to_string(&files[0]).unwrap_or_default()));
        let headers = router::parse_headers(&std::fs::read_to_string(&files[1]).unwrap_or_default());
        let r = Arc::new((redirects, headers));
        c.insert(root.to_path_buf(), (stamp, r.clone()));
        r
    }

    fn handle(&self, rq: Request) {
        let method = rq.method().as_str().to_string();
        let url = rq.url().to_string();
        let header = |name: &str| rq.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string());
        let host = header("host").unwrap_or_default().to_ascii_lowercase();
        let (path, query) = url.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((url.clone(), String::new()));

        // only our own names, on our port (DNS rebinding can't borrow the server)
        let Some(label) = self.label_of(&host) else {
            let _ = rq.respond(text(421, "This server only answers for sites hosted by Repo Sync."));
            return;
        };
        if path == "/__reposync/ping" {
            let body = serde_json::json!({ "app": "Repo Sync", "version": crate::VERSION, "port": self.port }).to_string();
            let _ = rq.respond(with_type(Response::from_string(body), "application/json"));
            return;
        }
        if label.is_empty() {
            let _ = rq.respond(self.index_page());
            return;
        }
        let Some(view) = self.sites.view(&label) else {
            let _ = rq.respond(html_page(404, "No site here", &format!("<p>Repo Sync hosts no site at <b>{}</b>. It may have been removed, or its version deleted.</p><p><a href=\"http://localhost:{}/\">Every hosted site</a></p>", esc(&host), self.port)));
            return;
        };
        // pages on the internet can't read (or probe) what's hosted here
        let origin = header("origin").unwrap_or_default();
        let ours = |o: &str| self.label_of(o.split_once("://").map(|(_, h)| h).unwrap_or("")).is_some();
        let site_ctx = header("sec-fetch-site").unwrap_or_default();
        let mode = header("sec-fetch-mode").unwrap_or_default();
        if site_ctx == "cross-site" && mode != "navigate" && !ours(&origin) {
            self.note(&view.project, &method, &url, 403, "refused: asked by a page on another site");
            let _ = rq.respond(text(403, "Refused: only pages hosted by Repo Sync can load this."));
            return;
        }
        let cors = (!origin.is_empty() && ours(&origin)).then_some(origin.clone());

        if method == "POST" && path == "/__reposync/csp" {
            self.csp_report(rq, &view);
            return;
        }
        if method != "GET" && method != "HEAD" {
            self.note(&view.project, &method, &url, 405, "");
            let _ = rq.respond(text(405, "Only GET and HEAD.").with_header(h("Allow", "GET, HEAD")));
            return;
        }
        if path == "/__reposync/reset" {
            let body = format!("<p>This site's saved data (storage, cookies and cache) on this computer has been cleared.</p><p><a href=\"{}\">Back to the site</a></p>", esc(&view.base));
            let r = html_page(200, "Site data cleared", &body).with_header(h("Clear-Site-Data", "\"cache\", \"cookies\", \"storage\""));
            self.note(&view.project, &method, &url, 200, "site data cleared");
            let _ = rq.respond(r);
            return;
        }
        if let Some(rest) = path.strip_prefix(ROUTE) {
            let r = self.saved_file(&view, rest, &query, &method, &url, cors.as_deref());
            let _ = rq.respond(r);
            return;
        }

        let rules = self.rules_of(&view.root);
        let wants_html = header("accept").map(|a| a.contains("text/html")).unwrap_or(false);
        let site = router::Site { root: &view.root, base: &view.base, hide: view.hide, spa: view.spa, markdown: view.markdown, redirects: &rules.0 };
        let mut decision = router::decide(&site, &path, &query, wants_html);
        // a captured website: a page or file missing from the capture is fetched from the live site
        if let (Decision::NotFound(_), Some(from)) = (&decision, &view.record_from) {
            if !view.block_internet {
                let _g = self.fetching.lock().unwrap();
                if let Some(rel) = super::mirror::record(&view.root, from, &path, &query, &view.base, &self.sites.client()) {
                    self.note(&view.project, &method, &url, 200, "saved from the live site");
                    decision = Decision::File(view.root.join(rel));
                }
            }
        }
        let rel_for_headers = path.strip_prefix(view.base.as_str()).map(|r| format!("/{r}")).unwrap_or_else(|| path.clone());
        let mut extra = router::headers_for(&rules.1, &rel_for_headers);
        extra.extend(view.headers.iter().cloned());
        let range = header("range");
        let inm = header("if-none-match");
        match decision {
            Decision::Redirect(loc, code) => {
                self.note(&view.project, &method, &url, code, &format!("to {loc}"));
                let _ = rq.respond(Response::empty(code).with_header(h("Location", &loc)).with_header(h("Cache-Control", "no-cache")));
            }
            Decision::Bad => {
                self.note(&view.project, &method, &url, 400, "not an address of a file");
                let _ = rq.respond(text(400, "Not an address of a file."));
            }
            Decision::NotFound(page) => {
                self.note(&view.project, &method, &url, 404, "");
                let r = match page {
                    Some(p) => self.file_response(&view, &p, 404, None, None, &extra, cors.as_deref()),
                    None => html_page(404, "Not found", "<p>There's no page at this address in the site.</p>"),
                };
                let _ = rq.respond(r);
            }
            Decision::Markdown(p) => {
                self.note(&view.project, &method, &url, 200, "Markdown shown as a page");
                let body = markdown_page(&p);
                let mut r = with_type(Response::from_string(body), "text/html; charset=utf-8").with_header(h("Cache-Control", "no-cache"));
                r = self.policy(r, &view);
                let _ = rq.respond(r);
            }
            Decision::File(p) => {
                let r = self.file_response(&view, &p, 200, range.as_deref(), inm.as_deref(), &extra, cors.as_deref());
                let status = r.status_code().0;
                self.note(&view.project, &method, &url, status, "");
                let _ = rq.respond(r);
            }
        }
    }

    /// "" for the server's own address, the site's label for `<label>.localhost`,
    /// None for any other name or port.
    fn label_of(&self, host: &str) -> Option<String> {
        let (name, port) = match host.rsplit_once(':') {
            Some((n, p)) if !n.ends_with(']') || host.starts_with('[') => (n, p.parse::<u16>().ok()?),
            _ => return None,
        };
        if port != self.port {
            return None;
        }
        let name = name.trim_end_matches('.');
        if matches!(name, "localhost" | "127.0.0.1" | "[::1]") {
            return Some(String::new());
        }
        let label = name.strip_suffix(".localhost")?;
        (!label.is_empty() && label.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))).then(|| label.to_string())
    }

    fn index_page(&self) -> Response<Cursor<Vec<u8>>> {
        let mut list = self.sites.list();
        list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        let items: Vec<String> = list.iter().map(|s| format!("<li><a href=\"http://{}.localhost:{}{}\">{}</a> <small>{}.localhost</small></li>", s.label, self.port, s.base, esc(&s.title), s.label)).collect();
        html_page(200, "Sites hosted by Repo Sync", &if items.is_empty() { "<p>No sites yet. Open one from a project's page.</p>".to_string() } else { format!("<ul>{}</ul>", items.join("")) })
    }

    fn csp_report(&self, mut rq: Request, view: &View) {
        let mut body = vec![];
        let _ = rq.as_reader().take(64 * 1024).read_to_end(&mut body);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        let r = &v["csp-report"];
        let blocked = r["blocked-uri"].as_str().unwrap_or("");
        if !blocked.is_empty() && !blocked.starts_with("data") && !blocked.starts_with("blob") && blocked != "inline" && blocked != "eval" {
            self.note(&view.project, "CSP", blocked, 0, if view.block_internet { "blocked: the page reached for the internet" } else { "the page reached the internet for this" });
        }
        let _ = rq.respond(Response::empty(204));
    }

    /// `/__reposync/v/<host>/<path>`: a saved copy of a file from another server.
    fn saved_file(&self, view: &View, rest: &str, query: &str, method: &str, url: &str, cors: Option<&str>) -> Response<Cursor<Vec<u8>>> {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.to_ascii_lowercase();
        let full = format!("https://{host}/{path}{}", if query.is_empty() { String::new() } else { format!("?{query}") });
        let lock = view.lock.as_ref().and_then(|p| self.lock_of(p));
        let mut found = lock.as_ref().and_then(|l| l.find(&full).cloned());
        let mut note = String::new();
        if found.is_none() && view.save_misses && !view.block_internet {
            if let Some(lp) = &view.lock {
                let _g = self.fetching.lock().unwrap();
                found = vendor::record(&full, lp, &view.store, &self.sites.client(), &view.allow);
                if found.is_some() {
                    note = "saved from the internet".into();
                }
            }
        }
        let Some(f) = found else {
            self.note(&view.project, method, url, 504, &format!("not saved for offline use: {full}"));
            return text(504, "This file from another server wasn't saved for offline use.");
        };
        let Ok(mut bytes) = view.store.get(&f.blob) else {
            self.note(&view.project, method, url, 500, "the saved copy is missing");
            return text(500, "The saved copy of this file is missing from the library.");
        };
        let kind = mime::text_kind(&f.ctype);
        if kind == "js" || kind == "css" {
            let t = String::from_utf8_lossy(&bytes);
            let t = rewrite::root_relative_to_route(&t, kind, &host, ROUTE);
            let hosts = lock.as_ref().map(|l| l.hosts.clone()).unwrap_or_default();
            bytes = rewrite::swap_hosts(&t, &hosts).0.into_bytes();
        }
        self.note(&view.project, method, url, 200, &note);
        let mut r = with_type(Response::from_data(bytes), &f.ctype).with_header(h("Cache-Control", "no-cache")).with_header(h("X-Content-Type-Options", "nosniff"));
        if let Some(o) = cors {
            r = r.with_header(h("Access-Control-Allow-Origin", o)).with_header(h("Vary", "Origin"));
        }
        r
    }

    /// The site's pages can't reach the internet (Block the internet), or report when they do.
    fn policy<R: Read>(&self, r: Response<R>, view: &View) -> Response<R> {
        let csp = "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; report-uri /__reposync/csp";
        if view.block_internet {
            r.with_header(h("Content-Security-Policy", csp))
        } else {
            r.with_header(h("Content-Security-Policy-Report-Only", csp))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn file_response(&self, view: &View, p: &Path, status: u16, range: Option<&str>, inm: Option<&str>, extra: &[(String, String)], cors: Option<&str>) -> Response<Cursor<Vec<u8>>> {
        let ctype = mime::for_path(&p.to_string_lossy());
        let Ok(meta) = std::fs::metadata(p) else { return text(404, "Not found.") };
        let len = meta.len();
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        let lock = view.lock.as_ref().and_then(|l| self.lock_of(l));
        let hosts: Vec<String> = lock.as_ref().map(|l| l.hosts.clone()).unwrap_or_default();
        let rewrite_it = !hosts.is_empty() && mime::is_rewritable(ctype) && len < 20 << 20;
        let lock_stamp = view.lock.as_ref().and_then(|l| std::fs::metadata(l).and_then(|m| m.modified()).ok()).and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        let etag = format!("\"{len:x}-{mtime:x}{}\"", if rewrite_it { format!("-{lock_stamp:x}") } else { String::new() });
        let html = ctype.starts_with("text/html");
        let finish = |mut r: Response<Cursor<Vec<u8>>>| {
            r = r.with_header(h("Cache-Control", "no-cache")).with_header(h("X-Content-Type-Options", "nosniff")).with_header(h("ETag", &etag));
            if let Some(o) = cors {
                r = r.with_header(h("Access-Control-Allow-Origin", o)).with_header(h("Vary", "Origin"));
            }
            for (k, v) in extra {
                if let Ok(hh) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    r = r.with_header(hh);
                }
            }
            if html {
                r = self.policy(r, view);
            }
            r
        };
        if status == 200 && inm.is_some_and(|v| v.split(',').any(|t| t.trim() == etag)) {
            return finish(Response::from_data(vec![]).with_status_code(304));
        }
        if rewrite_it {
            let Ok(bytes) = std::fs::read(p) else { return text(500, "Couldn't read the file.") };
            let t = String::from_utf8_lossy(&bytes);
            let (mut out, changed) = rewrite::swap_hosts(&t, &hosts);
            if changed && html {
                out = rewrite::strip_integrity(&out);
            }
            return finish(with_type(Response::from_data(out.into_bytes()), ctype).with_status_code(status));
        }
        // a byte range (audio and video seek with these)
        if let (Some(rg), 200) = (range, status) {
            if let Some((start, end)) = parse_range(rg, len) {
                let mut f = match std::fs::File::open(p) {
                    Ok(f) => f,
                    Err(_) => return text(500, "Couldn't read the file."),
                };
                let mut buf = vec![0u8; (end - start + 1) as usize];
                if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err() {
                    return text(500, "Couldn't read the file.");
                }
                return finish(with_type(Response::from_data(buf), ctype).with_status_code(206).with_header(h("Content-Range", &format!("bytes {start}-{end}/{len}"))).with_header(h("Accept-Ranges", "bytes")));
            }
            return finish(Response::from_data(vec![]).with_status_code(416).with_header(h("Content-Range", &format!("bytes */{len}"))));
        }
        match std::fs::read(p) {
            Ok(bytes) => finish(with_type(Response::from_data(bytes), ctype).with_status_code(status).with_header(h("Accept-Ranges", "bytes"))),
            Err(_) => text(500, "Couldn't read the file."),
        }
    }
}

/// One range from a `Range: bytes=…` header, as inclusive offsets; None if it
/// can't be met (or asks for several ranges).
pub fn parse_range(v: &str, len: u64) -> Option<(u64, u64)> {
    let spec = v.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        return Some((len.saturating_sub(n), len - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
    (start <= end && start < len).then_some((start, end))
}

fn h(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap_or_else(|_| Header::from_bytes(&b"X-Bad"[..], &b"1"[..]).unwrap())
}

fn with_type<R: Read>(r: Response<R>, ctype: &str) -> Response<R> {
    r.with_header(h("Content-Type", ctype))
}

fn text(code: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    with_type(Response::from_string(msg), "text/plain; charset=utf-8").with_status_code(StatusCode(code))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html_page(code: u16, title: &str, body: &str) -> Response<Cursor<Vec<u8>>> {
    let page = format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>{t}</title>\
<style>body{{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:3rem auto;padding:0 1rem;color:#1d2433}}h1{{font-size:1.4rem}}a{{color:#1c5aa6}}small{{color:#667}}@media(prefers-color-scheme:dark){{body{{background:#14171c;color:#dde3ea}}a{{color:#7fb0ef}}}}</style>\
<h1>{t}</h1>{body}<p><small>Hosted on this computer by Repo Sync</small></p></html>",
        t = esc(title)
    );
    with_type(Response::from_string(page), "text/html; charset=utf-8").with_status_code(StatusCode(code))
}

/// A Markdown file as a page, as GitHub Pages shows one (links to other .md files lead to their pages).
pub fn markdown_page(p: &Path) -> String {
    let md = std::fs::read_to_string(p).unwrap_or_default();
    // front matter isn't part of the page
    let body = if let Some(rest) = md.strip_prefix("---\n").or_else(|| md.strip_prefix("---\r\n")) { rest.split_once("\n---").map(|(_, b)| b.trim_start_matches(['-', '\r', '\n'])).unwrap_or(rest) } else { &md };
    let mut opts = pulldown_cmark::Options::empty();
    opts.insert(pulldown_cmark::Options::ENABLE_TABLES);
    opts.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    opts.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, pulldown_cmark::Parser::new_ext(body, opts));
    let md_link = regex::Regex::new(r#"href="([^":]+?)\.md(#[^"]*)?""#).unwrap();
    let out = md_link.replace_all(&out, |c: &regex::Captures| {
        let target = &c[1];
        let page = if target.eq_ignore_ascii_case("readme") || target.to_ascii_lowercase().ends_with("/readme") { format!("{}", target.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_else(|| "./".into())) } else { format!("{target}.html") };
        format!("href=\"{page}{}\"", c.get(2).map(|m| m.as_str()).unwrap_or(""))
    });
    let title = body.lines().find(|l| l.starts_with("# ")).map(|l| l[2..].trim().to_string()).unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{t}</title>\
<style>body{{font:16px/1.6 -apple-system,BlinkMacSystemFont,\"Segoe UI\",Helvetica,Arial,sans-serif;max-width:52rem;margin:2rem auto;padding:0 1rem;color:#1f2328}}\
pre,code{{font-family:ui-monospace,Consolas,monospace;background:#f6f8fa}}pre{{padding:1rem;overflow:auto}}img{{max-width:100%}}table{{border-collapse:collapse}}td,th{{border:1px solid #d0d7de;padding:4px 10px}}a{{color:#0969da}}\
@media(prefers-color-scheme:dark){{body{{background:#0d1117;color:#e6edf3}}pre,code{{background:#161b22}}a{{color:#4493f8}}}}</style>\n{out}</html>",
        t = esc(&title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    struct One {
        root: PathBuf,
        lock: PathBuf,
        store: Store,
    }
    impl Sites for One {
        fn view(&self, label: &str) -> Option<View> {
            (label == "demo.me" || label == "abc1234.demo.me").then(|| View {
                project: "github.com/me/demo".into(),
                title: "Demo".into(),
                root: self.root.clone(),
                base: "/demo/".into(),
                hide: Hide::Artifact,
                spa: false,
                markdown: false,
                lock: Some(self.lock.clone()),
                store: self.store.clone(),
                allow: Allow::default(),
                headers: vec![("X-Extra".into(), "1".into())],
                block_internet: false,
                save_misses: false,
                record_from: None,
            })
        }
        fn list(&self) -> Vec<Listed> {
            vec![Listed { label: "demo.me".into(), title: "Demo".into(), base: "/demo/".into() }]
        }
        fn client(&self) -> Arc<Client> {
            Arc::new(Client::new())
        }
    }

    fn get(port: u16, host: &str, path: &str, extra: &str) -> (u16, String, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{extra}\r\n").unwrap();
        let mut buf = vec![];
        s.read_to_end(&mut buf).unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&buf[..split]).to_string();
        let code = head.split(' ').nth(1).unwrap().parse().unwrap();
        (code, head, buf[split + 4..].to_vec())
    }

    #[test]
    fn serves_sites_by_name_with_saved_files() {
        let d = std::env::temp_dir().join(format!("reposync-server-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let root = d.join("site");
        std::fs::create_dir_all(root.join("audio")).unwrap();
        std::fs::write(root.join("index.html"), r#"<script type="importmap">{"imports":{"three":"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js"}}</script><script src="https://cdn.jsdelivr.net/npm/x@1/x.js" integrity="sha384-x"></script>"#).unwrap();
        std::fs::write(root.join("audio/a.mp3"), b"0123456789").unwrap();
        let store = Store::new(&d.join("lib"));
        let sha = store.put(b"export const REVISION = '170'; import '/npm/dep@1/+esm';").unwrap();
        let lock = d.join("lock.json");
        let mut l = Lock::default();
        l.hosts = vec!["cdn.jsdelivr.net".into()];
        l.files.insert("https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js".into(), vendor::Locked { blob: sha, ctype: "text/javascript".into(), bytes: 10, ..Default::default() });
        vendor::write_lock(&lock, &l).unwrap();
        let srv = start(None, true, Arc::new(One { root, lock, store })).unwrap();
        let p = srv.port;
        let host = format!("demo.me.localhost:{p}");

        let (c, head, body) = get(p, &host, "/demo/", "");
        assert_eq!(c, 200);
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("\"three\":\"/__reposync/v/cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js\""), "{body}");
        assert!(!body.contains("integrity"));
        assert!(head.to_ascii_lowercase().contains("content-security-policy-report-only") && head.contains("X-Extra: 1"));
        assert_eq!(get(p, &host, "/demo", "").0, 301);
        assert_eq!(get(p, &host, "/", "").0, 302);

        let (c, _, body) = get(p, &host, "/__reposync/v/cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js", "");
        assert_eq!(c, 200);
        assert!(String::from_utf8(body).unwrap().contains("import '/__reposync/v/cdn.jsdelivr.net/npm/dep@1/+esm'"));
        assert_eq!(get(p, &host, "/__reposync/v/cdn.jsdelivr.net/npm/missing.js", "").0, 504);

        let (c, head, body) = get(p, &host, "/demo/audio/a.mp3", "Range: bytes=2-5\r\n");
        assert_eq!((c, body.as_slice()), (206, &b"2345"[..]));
        assert!(head.contains("Content-Range: bytes 2-5/10"));
        let (c, head, _) = get(p, &host, "/demo/audio/a.mp3", "");
        assert!(c == 200 && head.contains("Content-Length: 10") && head.contains("audio/mpeg"), "{head}");
        let etag = head.lines().find(|l| l.starts_with("ETag")).unwrap().split_once(": ").unwrap().1.to_string();
        assert_eq!(get(p, &host, "/demo/audio/a.mp3", &format!("If-None-Match: {etag}\r\n")).0, 304);

        // only our names, on our port
        assert_eq!(get(p, "evil.example.com", "/demo/", "").0, 421);
        assert_eq!(get(p, &format!("demo.me.localhost:{}", p.wrapping_add(1)), "/demo/", "").0, 421);
        assert_eq!(get(p, &format!("nothing.localhost:{p}"), "/", "").0, 404);
        assert_eq!(get(p, &host, "/demo/x", "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: cors\r\nOrigin: https://evil.example\r\n").0, 403);
        assert_eq!(get(p, &host, "/demo/x", "Sec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\n").0, 404, "a link from elsewhere still opens the site");
        let (c, head, _) = get(p, &host, "/demo/audio/a.mp3", &format!("Origin: http://abc1234.demo.me.localhost:{p}\r\nSec-Fetch-Site: same-site\r\n"));
        assert!(c == 200 && head.contains(&format!("Access-Control-Allow-Origin: http://abc1234.demo.me.localhost:{p}")));
        assert_eq!(get(p, &host, "/demo/..%2f..%2fetc%2fpasswd", "").0, 400);
        let (c, _, body) = get(p, &format!("localhost:{p}"), "/", "");
        assert!(c == 200 && String::from_utf8_lossy(&body).contains("demo.me.localhost"));
        let (c, _, body) = get(p, &host, "/__reposync/ping", "");
        assert!(c == 200 && String::from_utf8_lossy(&body).contains("Repo Sync"));
        assert!(!srv.log_of("github.com/me/demo").is_empty());
        srv.stop();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=2-5", 10), Some((2, 5)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=8-100", 10), Some((8, 9)));
        assert_eq!(parse_range("bytes=10-", 10), None);
        assert_eq!(parse_range("bytes=0-1,4-5", 10), None);
        assert_eq!(parse_range("items=0-1", 10), None);
    }
}
