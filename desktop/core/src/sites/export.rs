//! Packaging a version of a site as a repo of its own: its files, the saved
//! copies of files from other servers in `vendor/` (with the site's addresses
//! pointed at them), a README and `.nojekyll`; as a folder, a ZIP or a Git
//! folder, which can be pushed to an empty GitHub repo (docs/PLAN.md, "Export").

use super::rewrite;
use super::vendor::{Lock, Store};
use super::{mime, scan};
use crate::http::Progress;
use crate::util::{now_iso, write_atomic};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use url::Url;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Folder,
    Zip,
    Git,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub format: Format,
    /// The folder (or ZIP file) to make; it must not exist yet.
    pub dest: PathBuf,
    /// Put the saved files from other servers in `vendor/` (default true).
    pub vendor: Option<bool>,
    /// Add a GitHub Pages workflow that publishes the folder as it is.
    pub pages_workflow: bool,
    pub title: String,
    /// Where the site came from (its repo or address), for the README.
    pub source: String,
    /// Which version, for the README.
    pub version: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Exported {
    pub path: String,
    pub files: usize,
    pub bytes: u64,
    pub vendored: usize,
    pub warnings: Vec<String>,
}

const WARN_FILE: u64 = 50 << 20;
const MAX_FILE: u64 = 100 << 20;

/// Make the export of the site in `root` (a version's published folder).
pub fn export(root: &Path, lock: Option<&Lock>, store: &Store, opts: &Options, progress: &Progress) -> Result<Exported> {
    if opts.dest.as_os_str().is_empty() {
        bail!("Choose where to save the export.");
    }
    if opts.dest.exists() {
        bail!("{} already exists. Choose a new name.", opts.dest.display());
    }
    let folder = match opts.format {
        Format::Zip => opts.dest.with_extension(format!("partial-{}", std::process::id())),
        _ => opts.dest.clone(),
    };
    std::fs::create_dir_all(&folder).with_context(|| format!("couldn't create {}", folder.display()))?;
    let made = (|| -> Result<Exported> {
        let mut out = Exported { path: opts.dest.to_string_lossy().into_owned(), ..Default::default() };
        // the saved copies, by address, and where each goes in vendor/
        let mut map: Vec<(String, String)> = vec![];
        if opts.vendor.unwrap_or(true) {
            if let Some(l) = lock {
                for (url, f) in &l.files {
                    let Some(rel) = vendor_path(url, &f.ctype) else { continue };
                    let Ok(bytes) = store.get(&f.blob) else {
                        out.warnings.push(format!("The saved copy of {url} is missing; the export loads it from the internet."));
                        continue;
                    };
                    let kind = mime::text_kind(&f.ctype);
                    let bytes = if kind == "js" || kind == "css" {
                        let depth = rel.matches('/').count();
                        let up = "../".repeat(depth);
                        let host = Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase())).unwrap_or_default();
                        let t = rewrite::root_relative_to_route(&String::from_utf8_lossy(&bytes), kind, &host, &up);
                        t.into_bytes()
                    } else {
                        bytes
                    };
                    write_atomic(&folder.join("vendor").join(&rel), &bytes)?;
                    out.vendored += 1;
                    out.bytes += bytes.len() as u64;
                    map.push((url.clone(), format!("vendor/{rel}")));
                }
            }
        }
        // longest first, so ".../three.module.js" isn't caught by a shorter address
        map.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        // vendor files point at each other too
        if !map.is_empty() {
            for (_, rel) in map.clone() {
                let p = folder.join(&rel);
                let kind = mime::text_kind(mime::for_path(&rel));
                if kind != "js" && kind != "css" {
                    continue;
                }
                if let Ok(t) = std::fs::read_to_string(&p) {
                    let (o, changed) = point_at(&t, &map, &rel);
                    if changed {
                        write_atomic(&p, o.as_bytes())?;
                    }
                }
            }
        }
        // the site's own files
        let files = walk(root);
        for rel in &files {
            if progress.stopped() {
                bail!("Stopped.");
            }
            let from = root.join(rel);
            let to = folder.join(rel);
            let len = std::fs::metadata(&from).map(|m| m.len()).unwrap_or(0);
            if len > MAX_FILE {
                out.warnings.push(format!("{rel} is over 100 MB, which GitHub refuses; it was left out."));
                continue;
            }
            if len > WARN_FILE {
                out.warnings.push(format!("{rel} is over 50 MB; GitHub accepts it with a warning."));
            }
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            let ctype = mime::for_path(rel);
            if !map.is_empty() && mime::is_rewritable(ctype) && len < 20 << 20 {
                let t = std::fs::read_to_string(&from).unwrap_or_default();
                let (mut o, changed) = point_at(&t, &map, rel);
                if changed {
                    if ctype.starts_with("text/html") {
                        o = rewrite::strip_integrity(&o);
                    }
                    std::fs::write(&to, o.as_bytes())?;
                    out.bytes += o.len() as u64;
                    out.files += 1;
                    continue;
                }
            }
            std::fs::copy(&from, &to).with_context(|| format!("couldn't copy {rel}"))?;
            out.bytes += len;
            out.files += 1;
        }
        // notes, and what GitHub Pages needs
        if !folder.join(".nojekyll").exists() {
            std::fs::write(folder.join(".nojekyll"), b"")?;
        }
        let notes = readme(opts, &out, lock);
        let name = if folder.join("README.md").exists() { "EXPORTED.md" } else { "README.md" };
        write_atomic(&folder.join(name), notes.as_bytes())?;
        if opts.pages_workflow {
            write_atomic(&folder.join(".github/workflows/pages.yml"), PAGES_YML.as_bytes())?;
        }
        Ok(out)
    })();
    let mut out = match made {
        Ok(o) => o,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(e);
        }
    };
    match opts.format {
        Format::Folder => {}
        Format::Git => {
            if let Err(e) = git_init(&folder, &opts.title) {
                out.warnings.push(format!("The folder was made, but Git couldn't record it: {e:#}"));
            }
        }
        Format::Zip => {
            let r = zip_folder(&folder, &opts.dest);
            let _ = std::fs::remove_dir_all(&folder);
            r?;
        }
    }
    Ok(out)
}

/// Where a saved address goes under `vendor/`: "<host>/<path>", with a query (Google
/// Fonts' "css2?family=…") folded into the name.
pub fn vendor_path(url: &str, ctype: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    let mut path = crate::util::percent_decode(u.path().trim_start_matches('/'));
    if path.is_empty() || path.ends_with('/') {
        path.push_str("index");
    }
    if let Some(q) = u.query() {
        let h = super::vendor::hex(&<sha2::Sha256 as sha2::Digest>::digest(q.as_bytes()));
        path = format!("{path}__{}", &h[..10]);
    }
    let last = path.rsplit('/').next().unwrap_or("");
    if !last.contains('.') || u.query().is_some() {
        let ext = mime::extension_for(ctype);
        if !last.ends_with(ext) {
            path.push_str(ext);
        }
    }
    let ok = super::router::clean(&crate::util::percent_encode(&path)).is_some_and(|c| c == path);
    ok.then(|| format!("{host}/{path}"))
}

/// `text` (the file at `rel` in the export) with every saved address pointed at
/// its copy, relative to the file.
fn point_at(text: &str, map: &[(String, String)], rel: &str) -> (String, bool) {
    if !text.contains("//") {
        return (text.to_string(), false);
    }
    let up = "../".repeat(rel.matches('/').count());
    let mut out = text.to_string();
    let mut changed = false;
    // a folder of saved files (an import map's "three/addons/" prefix) points at its folder in vendor/
    let mut pairs: Vec<(String, String)> = map.to_vec();
    for u in scan::absolute_urls(text) {
        if !u.ends_with('/') || pairs.iter().any(|(k, _)| *k == u) || !map.iter().any(|(k, _)| k.starts_with(&u) && !k.contains('?')) {
            continue;
        }
        if let Ok(p) = Url::parse(&u) {
            let host = p.host_str().unwrap_or("").to_ascii_lowercase();
            pairs.push((u.clone(), format!("vendor/{host}/{}", crate::util::percent_decode(p.path().trim_start_matches('/')))));
        }
    }
    pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (url, to) in &pairs {
        let target = format!("{up}{to}");
        let bare = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
        let forms = [url.clone(), url.replace('&', "&amp;"), format!("http://{bare}"), format!("//{bare}")];
        for f in forms.iter() {
            if out.contains(f.as_str()) {
                out = replace_whole(&out, f, &target);
                changed = true;
            }
        }
    }
    (out, changed)
}

/// Replace `from` only where the address ends there (not a longer address).
fn replace_whole(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in text.match_indices(from) {
        if i < last {
            continue;
        }
        let next = text[i + from.len()..].chars().next();
        let ends = next.is_none_or(|c| matches!(c, '"' | '\'' | '`' | ')' | ' ' | '\n' | '\r' | '\t' | '<' | '>' | '#' | ',' | ';'));
        let before = text[..i].chars().last();
        let starts = from.starts_with("http") || matches!(before, None | Some('"' | '\'' | '(' | '=' | ' ' | '`' | ',' | '\n'));
        if ends && starts && !(from.starts_with("//") && text[..i].ends_with(':')) {
            out.push_str(&text[last..i]);
            out.push_str(to);
            last = i + from.len();
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Every file under `root` (relative, with "/"), but `.git/`.
fn walk(root: &Path) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => {
                    if let Ok(r) = e.path().strip_prefix(root) {
                        out.push(r.to_string_lossy().replace('\\', "/"));
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

fn readme(opts: &Options, out: &Exported, lock: Option<&Lock>) -> String {
    let title = if opts.title.is_empty() { "A site" } else { opts.title.as_str() };
    let mut s = format!("# {title}\n\nExported by Repo Sync on {}.\n\n", now_iso());
    if !opts.source.is_empty() {
        s.push_str(&format!("- From: {}\n", opts.source));
    }
    if !opts.version.is_empty() {
        s.push_str(&format!("- Version: {}\n", opts.version));
    }
    s.push_str(&format!("- {} files of the site", out.files));
    if out.vendored > 0 {
        s.push_str(&format!(", and {} files from other servers in `vendor/`", out.vendored));
    }
    s.push_str(".\n\n## Opening it\n\nServe the folder with any static web server, for example `python -m http.server`, and open the address it gives. \
GitHub Pages can publish it as it is (`.nojekyll` is included).\n");
    if out.vendored > 0 {
        s.push_str("\n## Files from other servers\n\nScripts, styles and fonts the site loads from CDNs were saved in `vendor/<server>/`, and the site's addresses for them were changed to point there, so it works offline. \
`integrity` checks were taken off pages that use them (the saved files are the same, but their addresses changed). \
An address a script builds while it runs isn't changed, and still loads from the internet.\n");
    }
    if let Some(l) = lock {
        if !l.needs_internet.is_empty() {
            s.push_str("\n## Still online\n\nThese servers are used by the site and weren't saved (APIs, analytics and the like):\n\n");
            for n in &l.needs_internet {
                s.push_str(&format!("- {} ({} addresses, for example {})\n", n.host, n.count, n.example));
            }
        }
    }
    if !out.warnings.is_empty() {
        s.push_str("\n## Notes\n\n");
        for w in &out.warnings {
            s.push_str(&format!("- {w}\n"));
        }
    }
    s
}

const PAGES_YML: &str = "name: Publish to GitHub Pages\non:\n  push:\n    branches: [main]\n  workflow_dispatch:\npermissions:\n  contents: read\n  pages: write\n  id-token: write\nconcurrency:\n  group: pages\n  cancel-in-progress: true\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    environment:\n      name: github-pages\n      url: ${{ steps.deployment.outputs.page_url }}\n    steps:\n      - uses: actions/checkout@v4\n      - uses: actions/configure-pages@v5\n      - uses: actions/upload-pages-artifact@v3\n        with:\n          path: .\n      - id: deployment\n        uses: actions/deploy-pages@v4\n";

fn zip_folder(folder: &Path, dest: &Path) -> Result<()> {
    let tmp = dest.with_extension(format!("zip-partial-{}", std::process::id()));
    let f = std::fs::File::create(&tmp).with_context(|| format!("couldn't create {}", dest.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let top = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "site".into());
    let r = (|| -> Result<()> {
        for rel in walk(folder) {
            z.start_file(format!("{top}/{rel}"), opts)?;
            let mut src = std::fs::File::open(folder.join(&rel))?;
            std::io::copy(&mut src, &mut z)?;
        }
        z.finish()?.flush()?;
        Ok(())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir).env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

fn run(mut c: Command, what: &str) -> Result<String> {
    let o = c.output().with_context(|| format!("couldn't run git to {what} (is Git installed?)"))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        bail!("git couldn't {what}: {}", err.trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Make `folder` a Git repository with one commit of everything in it.
pub fn git_init(folder: &Path, title: &str) -> Result<()> {
    let mut c = git(folder);
    c.args(["init", "-q", "-b", "main"]);
    run(c, "start a repository")?;
    let mut c = git(folder);
    c.args(["add", "-A"]);
    run(c, "add the files")?;
    let mut c = git(folder);
    let msg = if title.is_empty() { "Site exported by Repo Sync".to_string() } else { format!("{title}, exported by Repo Sync") };
    c.args(["-c", "user.name=Repo Sync", "-c", "user.email=repo-sync@users.noreply.github.com", "commit", "-q", "-m", &msg]);
    run(c, "record the files")?;
    Ok(())
}

/// Push the Git folder `folder` to `remote` (an https address of an empty repo),
/// signing in with `token` (passed to git in its environment, never its command line).
pub fn push(folder: &Path, remote: &str, token: &str) -> Result<()> {
    let u = Url::parse(remote).with_context(|| format!("{remote} isn't the address of a repo"))?;
    if u.scheme() != "https" || u.host_str().is_none() {
        bail!("Use the repo's https address (https://github.com/you/name).");
    }
    if !folder.join(".git").is_dir() {
        bail!("{} isn't a Git folder.", folder.display());
    }
    let mut c = git(folder);
    c.args(["push", remote, "main"]);
    if !token.is_empty() {
        let host = u.host_str().unwrap_or("");
        c.env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("http.https://{host}/.extraheader"))
            .env("GIT_CONFIG_VALUE_0", format!("AUTHORIZATION: basic {}", base64(format!("x-access-token:{token}").as_bytes())));
    }
    run(c, "push").map(|_| ())
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The addresses a page uses on the servers a lock covers, for a check that the
/// export points at its copies (used by the tests).
pub fn remaining(text: &str, lock: &Lock) -> Vec<String> {
    scan::absolute_urls(text).into_iter().filter(|u| lock.files.contains_key(u)).collect::<BTreeSet<_>>().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sites::vendor::Locked;

    #[test]
    fn base64_matches() {
        assert_eq!(base64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn vendor_paths() {
        assert_eq!(vendor_path("https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js", "text/javascript").as_deref(), Some("cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js"));
        let f = vendor_path("https://fonts.googleapis.com/css2?family=A&display=swap", "text/css").unwrap();
        assert!(f.starts_with("fonts.googleapis.com/css2__") && f.ends_with(".css"), "{f}");
        assert_eq!(vendor_path("https://esm.sh/npm/x@1/+esm", "text/javascript").as_deref(), Some("esm.sh/npm/x@1/+esm.js"));
    }

    #[test]
    fn an_export_points_at_its_copies() {
        let d = std::env::temp_dir().join(format!("reposync-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let site = d.join("site");
        std::fs::create_dir_all(site.join("js")).unwrap();
        std::fs::write(site.join("index.html"), r#"<link href="https://fonts.googleapis.com/css2?family=A&amp;display=swap" rel=stylesheet><script src="https://cdn.jsdelivr.net/npm/x@1/x.js" integrity="sha384-x"></script><a href="https://cdn.jsdelivr.net/npm/x@1/x.js.map">map</a><script type="importmap">{"imports":{"x/":"https://cdn.jsdelivr.net/npm/x@1/"}}</script>"#).unwrap();
        std::fs::write(site.join("js/a.js"), r#"import "https://cdn.jsdelivr.net/npm/x@1/x.js";"#).unwrap();
        let store = Store::new(&d.join("lib"));
        let mut lock = Lock::default();
        let put = |s: &str| store.put(s.as_bytes()).unwrap();
        lock.files.insert("https://cdn.jsdelivr.net/npm/x@1/x.js".into(), Locked { blob: put("console.log(1)"), ctype: "text/javascript".into(), ..Default::default() });
        lock.files.insert("https://fonts.googleapis.com/css2?family=A&display=swap".into(), Locked { blob: put("@font-face{src:url(https://fonts.gstatic.com/a.woff2)}"), ctype: "text/css".into(), ..Default::default() });
        lock.files.insert("https://fonts.gstatic.com/a.woff2".into(), Locked { blob: put("font"), ctype: "font/woff2".into(), ..Default::default() });
        let opts = Options { dest: d.join("out"), title: "T".into(), ..Default::default() };
        let out = export(&site, Some(&lock), &store, &opts, &Progress::default()).unwrap();
        assert_eq!(out.vendored, 3);
        let html = std::fs::read_to_string(d.join("out/index.html")).unwrap();
        assert!(html.contains(r#"src="vendor/cdn.jsdelivr.net/npm/x@1/x.js""#), "{html}");
        assert!(html.contains("href=\"vendor/fonts.googleapis.com/css2__") && !html.contains("integrity"));
        assert!(html.contains("https://cdn.jsdelivr.net/npm/x@1/x.js.map"), "a longer address is left alone");
        assert!(html.contains(r#""x/":"vendor/cdn.jsdelivr.net/npm/x@1/""#), "a folder of saved files points at its folder: {html}");
        let js = std::fs::read_to_string(d.join("out/js/a.js")).unwrap();
        assert!(js.contains(r#"import "../vendor/cdn.jsdelivr.net/npm/x@1/x.js""#), "{js}");
        let css_rel = walk(&d.join("out")).into_iter().find(|r| r.starts_with("vendor/fonts.googleapis.com/")).unwrap();
        let css = std::fs::read_to_string(d.join("out").join(css_rel)).unwrap();
        assert!(css.contains("url(../../vendor/fonts.gstatic.com/a.woff2)"), "{css}");
        assert!(d.join("out/README.md").is_file() && d.join("out/.nojekyll").is_file());
        let zopts = Options { format: Format::Zip, dest: d.join("out.zip"), ..Default::default() };
        export(&site, Some(&lock), &store, &zopts, &Progress::default()).unwrap();
        assert!(d.join("out.zip").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }
}
