//! Command-line access to Repo Sync's backend (the same code, no window).
//!
//!   reposync-cli --version
//!   reposync-cli list [--library <dir>]
//!   reposync-cli add <link>... [--library <dir>]     (add projects and capture them)
//!   reposync-cli check-all [--library <dir>]         (check every project now; for a
//!                                                     scheduled task when the app is closed)
//!   reposync-cli mirror <address>... [--library <dir>] (save websites as projects)
//!   reposync-cli host [--port <n>] [--library <dir>]  (host the library's sites until stopped)
//!   reposync-cli site <project> [--library <dir>]     (what a project holds as a website)
//!   reposync-cli export <project> <dest> [--format folder|zip|git] [--version <list>/<folder>]
//!                     [--push <https address>] [--library <dir>]   (package a site as a repo)
//!   reposync-cli serve --ui <dir> --home <dir> [--library <dir>] [--port 8790] [--tick <seconds>]
//!
//! `serve` runs the app's commands behind a local web server, so the page can be
//! tested in an ordinary browser (tests/tauri_shim.js stands in for Tauri's `invoke`).

use anyhow::{bail, Context, Result};
use reposync_core::api::{App, AppPaths};
use reposync_core::config::AppConfig;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> Option<String> {
        let i = self.rest.iter().position(|a| a == name)?;
        if i + 1 >= self.rest.len() {
            return None;
        }
        let v = self.rest.remove(i + 1);
        self.rest.remove(i);
        Some(v)
    }
    fn need(&mut self, name: &str) -> Result<String> {
        self.flag(name).with_context(|| format!("{name} is needed"))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut a = Args { rest: std::env::args().skip(1).collect() };
    let cmd = if a.rest.is_empty() { String::new() } else { a.rest.remove(0) };
    match cmd.as_str() {
        "--version" | "-V" => {
            println!("reposync-cli {}", reposync_core::VERSION);
            Ok(())
        }
        "serve" => serve(a),
        "list" | "add" | "check-all" | "mirror" | "host" | "site" | "export" => {
            let app = local_app(&mut a)?;
            match cmd.as_str() {
                "list" => {
                    let v = app.call("repos", json!({})).map_err(anyhow::Error::msg)?;
                    for r in v["repos"].as_array().into_iter().flatten() {
                        println!(
                            "{}  {} versions, {} releases, last checked {}{}",
                            r["id"].as_str().unwrap_or(""),
                            r["snapshots"],
                            r["releases"],
                            r["last_checked"].as_str().filter(|s| !s.is_empty()).unwrap_or("never"),
                            r["error"].as_str().filter(|s| !s.is_empty()).map(|e| format!("  ({e})")).unwrap_or_default()
                        );
                    }
                    Ok(())
                }
                "add" | "mirror" => {
                    if a.rest.is_empty() {
                        bail!("give one or more links");
                    }
                    let v = app.call("add", json!({ "inputs": a.rest, "website": cmd == "mirror" })).map_err(anyhow::Error::msg)?;
                    for r in v.as_array().into_iter().flatten() {
                        if let Some(e) = r["error"].as_str() {
                            eprintln!("{}: {e}", r["input"].as_str().unwrap_or(""));
                        }
                    }
                    wait(&app)
                }
                "site" => {
                    let id = a.rest.first().context("give a project, such as github.com/owner/name")?;
                    let v = app.call("site_info", json!({ "id": id })).map_err(anyhow::Error::msg)?;
                    println!("{}", serde_json::to_string_pretty(&v)?);
                    Ok(())
                }
                "host" => {
                    let v = app.call("sites", json!({})).map_err(anyhow::Error::msg)?;
                    println!("Hosting on {}", v["index"].as_str().unwrap_or(""));
                    for s in v["sites"].as_array().into_iter().flatten() {
                        println!("  {}  {}", s["url"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or(""));
                    }
                    println!("Stop with Ctrl+C.");
                    loop {
                        std::thread::sleep(Duration::from_secs(3600));
                    }
                }
                "export" => {
                    let format = a.flag("--format").unwrap_or_else(|| "folder".into());
                    let version = a.flag("--version");
                    let push = a.flag("--push").unwrap_or_default();
                    let (Some(id), Some(dest)) = (a.rest.first().cloned(), a.rest.get(1).cloned()) else { bail!("give a project and where to save the export") };
                    let (coll, folder) = match version.as_deref().and_then(|v| v.split_once('/')) {
                        Some((c, f)) => (c.to_string(), f.to_string()),
                        None => {
                            let info = app.call("site_info", json!({ "id": id })).map_err(anyhow::Error::msg)?;
                            let m = &info["main"];
                            if m.is_null() {
                                bail!("{id} has no version that can be shown as a website");
                            }
                            (m["coll"].as_str().unwrap_or("").to_string(), m["folder"].as_str().unwrap_or("").to_string())
                        }
                    };
                    let dest = std::path::absolute(&dest)?;
                    let options = json!({ "format": format, "dest": dest, "pages_workflow": a.rest.iter().any(|x| x == "--pages-workflow") });
                    app.call("site_export", json!({ "id": id, "coll": coll, "folder": folder, "options": options, "push": push })).map_err(anyhow::Error::msg)?;
                    wait(&app)
                }
                _ => {
                    app.call("check_all", json!({})).map_err(anyhow::Error::msg)?;
                    wait(&app)
                }
            }
        }
        _ => bail!("usage: reposync-cli --version | list | add <link>... | check-all | mirror <address>... | host [--port <n>] | site <project> | export <project> <dest> | serve --ui <dir> --home <dir>  [--library <dir>]"),
    }
}

/// The app with this computer's config, or another library with --library.
fn local_app(a: &mut Args) -> Result<Arc<App>> {
    let config_dir = std::env::var_os("REPOSYNC_CONFIG").map(PathBuf::from).unwrap_or_else(default_config_dir);
    let lib = a.flag("--library");
    let port: Option<u16> = a.flag("--port").map(|p| p.parse()).transpose()?;
    if lib.is_some() || port.is_some() {
        let file = config_dir.join("config.json");
        let mut cfg = AppConfig::load(&file);
        if let Some(lib) = lib {
            cfg.set_library(Path::new(&lib));
        }
        if port.is_some() {
            cfg.site_port = port;
        }
        cfg.save(&file)?;
    }
    App::new(AppPaths { config_dir, library_url: "library://localhost/".into() }, Duration::from_secs(3600 * 24 * 365))
}

/// Where the desktop app keeps its config (Tauri's app_config_dir for the app's identifier).
fn default_config_dir() -> PathBuf {
    let id = "io.github.drflgd.reposync";
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(reposync_core::config::home_dir).join(id)
    } else if cfg!(target_os = "macos") {
        reposync_core::config::home_dir().join("Library/Application Support").join(id)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| reposync_core::config::home_dir().join(".config")).join(id)
    }
}

/// Print each finished job until none are left.
fn wait(app: &App) -> Result<()> {
    let mut seen = 0usize;
    let mut failed = false;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let j = app.call("jobs", json!({})).map_err(anyhow::Error::msg)?;
        let recent = j["recent"].as_array().cloned().unwrap_or_default();
        for f in recent.iter().take(recent.len().saturating_sub(seen)).rev() {
            println!("{}: {}", f["job"]["repo"].as_str().unwrap_or(""), f["message"].as_str().unwrap_or(""));
            failed |= !f["ok"].as_bool().unwrap_or(false);
        }
        seen = recent.len();
        if j["current"].is_null() && j["queued"].as_array().is_none_or(|q| q.is_empty()) {
            break;
        }
    }
    if failed {
        bail!("some checks didn't finish");
    }
    Ok(())
}

struct Http {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut std::net::TcpStream) -> Result<Http> {
    let mut buf = vec![];
    let mut chunk = [0u8; 65536];
    let head_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            bail!("connection closed");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 1 << 20 {
            bail!("headers too long");
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("/").to_string();
    let len: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Ok(Http { method, path, body })
}

fn respond(stream: &mut std::net::TcpStream, code: u16, ctype: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {code} {}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        if code < 400 { "OK" } else { "Error" },
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

fn content_type(path: &str) -> &'static str {
    reposync_core::sites::mime::for_path(path)
}

/// The app's commands over HTTP: POST /invoke/<command> (as tests/tauri_shim.js
/// sends them), GET /library/<path> for library files, everything else from --ui.
/// POST /test/pick queues the answer to the next folder dialog; GET /test/opened
/// lists what the page asked to open (folders, web links).
fn serve(mut a: Args) -> Result<()> {
    let ui = PathBuf::from(a.need("--ui")?);
    let home = PathBuf::from(a.need("--home")?);
    let port: u16 = a.flag("--port").map(|p| p.parse()).transpose()?.unwrap_or(8790);
    let tick: f64 = a.flag("--tick").map(|p| p.parse()).transpose()?.unwrap_or(30.0);
    let config_dir = home.join("config");
    if let Some(lib) = a.flag("--library") {
        let file = config_dir.join("config.json");
        let mut cfg = AppConfig::load(&file);
        cfg.set_library(Path::new(&lib));
        cfg.save(&file)?;
    }
    let app = App::new(AppPaths { config_dir, library_url: "/library/".into() }, Duration::from_secs_f64(tick))?;
    let picks: Arc<Mutex<Vec<String>>> = Arc::default();
    let opened: Arc<Mutex<Vec<Value>>> = Arc::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("serving on http://127.0.0.1:{port}/ (home {})", home.display());
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        let (app, ui, picks, opened) = (app.clone(), ui.clone(), picks.clone(), opened.clone());
        std::thread::spawn(move || {
            let Ok(req) = read_request(&mut stream) else { return };
            let path = req.path.split('?').next().unwrap_or("/").to_string();
            let r: Result<()> = (|| {
                if let Some(cmd) = path.strip_prefix("/invoke/") {
                    let args: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                    match cmd {
                        "api" => match app.call(args["cmd"].as_str().unwrap_or(""), args["args"].clone()) {
                            Ok(v) => respond(&mut stream, 200, "application/json", &serde_json::to_vec(&v)?),
                            Err(e) => {
                                eprintln!("{} {}: {e}", args["cmd"].as_str().unwrap_or(""), args["args"]);
                                respond(&mut stream, 400, "application/json", &serde_json::to_vec(&json!(e))?)
                            }
                        },
                        "pick_folder" => {
                            let next = {
                                let mut p = picks.lock().unwrap();
                                (!p.is_empty()).then(|| p.remove(0))
                            };
                            respond(&mut stream, 200, "application/json", &serde_json::to_vec(&json!(next))?)
                        }
                        "open_path" | "open_url" | "reveal" => {
                            opened.lock().unwrap().push(json!({ "cmd": cmd, "args": args }));
                            respond(&mut stream, 200, "application/json", b"null")
                        }
                        other => respond(&mut stream, 404, "application/json", &serde_json::to_vec(&json!(format!("unknown command {other}")))?),
                    }
                } else if path == "/test/pick" {
                    let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                    if let Some(p) = v["path"].as_str() {
                        picks.lock().unwrap().push(p.to_string());
                    }
                    respond(&mut stream, 200, "application/json", b"true")
                } else if path == "/test/opened" {
                    let v = serde_json::to_vec(&*opened.lock().unwrap())?;
                    respond(&mut stream, 200, "application/json", &v)
                } else if let Some(rel) = path.strip_prefix("/library/") {
                    match app.library_file(rel) {
                        Ok(b) => respond(&mut stream, 200, content_type(rel), &b),
                        Err(e) => respond(&mut stream, 404, "text/plain", e.to_string().as_bytes()),
                    }
                } else if req.method == "GET" {
                    let rel = if path == "/" { "index.html".to_string() } else { reposync_core::util::percent_decode(path.trim_start_matches('/')) };
                    let p = reposync_core::util::safe_rel(Path::new(&rel)).map(|r| ui.join(r));
                    match p.and_then(|p| std::fs::read(p).ok()) {
                        Some(b) => respond(&mut stream, 200, content_type(&rel), &b),
                        None => respond(&mut stream, 404, "text/plain", b"not found"),
                    }
                } else {
                    respond(&mut stream, 405, "text/plain", b"")
                }
            })();
            if let Err(e) = r {
                eprintln!("{path}: {e:#}");
            }
        });
    }
    Ok(())
}
