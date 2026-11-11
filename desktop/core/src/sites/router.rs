//! Which file answers a request, the way GitHub Pages decides it: the site under
//! its base path, a redirect to add the trailing slash to a folder, `x` served
//! from `x.html`, `404.html` with status 404, names matched case for case, and
//! (when asked) a single-page app's index.html for addresses it routes itself.
//! Netlify-style `_redirects` and `_headers` files are followed too.

use std::path::{Path, PathBuf};

/// Which files a site keeps out of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hide {
    /// An Actions deployment: `.git` and `.github` aren't in the artifact.
    Artifact,
    /// Published from a branch through Jekyll: names starting with `_` or `.`,
    /// node_modules, vendor and the Gemfile aren't published.
    Jekyll,
    /// A captured website: its `_mirror/` notes.
    Mirror,
}

/// One rule of a `_redirects` file: `from to [status]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redirect {
    pub from: String,
    pub to: String,
    pub status: u16,
}

/// One block of a `_headers` file: a path pattern and its headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderRule {
    pub pattern: String,
    pub headers: Vec<(String, String)>,
}

/// What the router needs to know about a site.
pub struct Site<'a> {
    pub root: &'a Path,
    /// "/name/" or "/".
    pub base: &'a str,
    pub hide: Hide,
    /// Unknown page addresses get index.html (a single-page app).
    pub spa: bool,
    /// Markdown pages are shown as HTML (Jekyll on a branch with no layouts).
    pub markdown: bool,
    pub redirects: &'a [Redirect],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Serve this file (200).
    File(PathBuf),
    /// Show this Markdown file as a page (200).
    Markdown(PathBuf),
    /// Send the browser to this address (301 or 302).
    Redirect(String, u16),
    /// 404, with the site's own 404 page when it has one.
    NotFound(Option<PathBuf>),
    /// 400: an address that can't name a file.
    Bad,
}

/// Decide what answers `path` (the request's path, still percent-encoded, no
/// query). `query` is kept on redirects; `wants_html` is the request's Accept.
pub fn decide(site: &Site, path: &str, query: &str, wants_html: bool) -> Decision {
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    let base = site.base;
    if base != "/" {
        if path == "/" {
            return Decision::Redirect(base.to_string(), 302);
        }
        if path == base.trim_end_matches('/') {
            return Decision::Redirect(format!("{base}{q}"), 301);
        }
    }
    let Some(rest) = path.strip_prefix(base) else { return Decision::NotFound(None) };
    let Some(mut rel) = clean(rest) else { return Decision::Bad };
    let not_found = || {
        let p = site.root.join("404.html");
        Decision::NotFound(p.is_file().then_some(p))
    };

    // _redirects: redirects and rewrites
    if let Some((to, status)) = redirect_for(site.redirects, &format!("/{rel}")) {
        match status {
            301 | 302 | 303 | 307 | 308 => {
                let loc = if to.contains("://") { to } else { format!("{}{}", base.trim_end_matches('/'), to) };
                return Decision::Redirect(loc, status);
            }
            404 => return not_found(),
            _ => match clean(to.trim_start_matches('/')) {
                Some(r) => rel = r,
                None => return Decision::Bad,
            },
        }
    }
    if hidden(site.hide, &rel) {
        return not_found();
    }
    let trailing = rel.is_empty() || rel.ends_with('/');
    let target = site.root.join(rel.trim_end_matches('/'));
    if !trailing && is_file(site.root, &rel) {
        return Decision::File(target);
    }
    if rel.is_empty() || is_dir(site.root, rel.trim_end_matches('/')) {
        let dir_rel = rel.trim_end_matches('/');
        let index = ["index.html", "index.htm"].iter().map(|i| join_rel(dir_rel, i)).find(|r| is_file(site.root, r));
        let md = if site.markdown { ["index.md", "README.md", "readme.md", "Readme.md"].iter().map(|i| join_rel(dir_rel, i)).find(|r| is_file(site.root, r)) } else { None };
        if index.is_some() || md.is_some() {
            if !trailing {
                return Decision::Redirect(format!("{path}/{q}"), 301);
            }
            return match index {
                Some(i) => Decision::File(site.root.join(i)),
                None => Decision::Markdown(site.root.join(md.unwrap())),
            };
        }
    }
    if !trailing {
        let html = format!("{rel}.html");
        if is_file(site.root, &html) {
            return Decision::File(site.root.join(html));
        }
        if site.markdown {
            let stem = rel.strip_suffix(".html").unwrap_or(&rel);
            let md = format!("{stem}.md");
            if is_file(site.root, &md) {
                return Decision::Markdown(site.root.join(md));
            }
        }
    }
    if site.spa && wants_html && !last_has_extension(&rel) {
        let index = site.root.join("index.html");
        if index.is_file() {
            return Decision::File(index);
        }
    }
    not_found()
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn last_has_extension(rel: &str) -> bool {
    rel.trim_end_matches('/').rsplit('/').next().is_some_and(|s| s.contains('.'))
}

/// The request path after the base, percent-decoded once and checked: None for
/// anything that could leave the site's folder or names no file on Windows.
pub fn clean(rest: &str) -> Option<String> {
    let decoded = percent_decode_bytes(rest)?;
    let s = String::from_utf8(decoded).ok()?;
    if s.contains(['\\', '\0', ':']) {
        return None;
    }
    let trailing = s.ends_with('/');
    let segs: Vec<&str> = s.split('/').collect();
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        if seg.is_empty() {
            // only a trailing slash may leave an empty segment
            if i == n - 1 && trailing || n == 1 {
                continue;
            }
            return None;
        }
        if *seg == "." || *seg == ".." || seg.ends_with('.') || seg.ends_with(' ') || seg.chars().any(|c| c.is_control()) {
            return None;
        }
        let upper = seg.to_ascii_uppercase();
        let stem = upper.split('.').next().unwrap_or("");
        if matches!(stem, "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit()) {
            return None;
        }
    }
    Some(s)
}

fn percent_decode_bytes(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

fn hidden(hide: Hide, rel: &str) -> bool {
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let name = segs.last().copied().unwrap_or("");
    if segs.iter().any(|s| *s == ".git") || matches!(name, "_redirects" | "_headers") && segs.len() == 1 {
        return true;
    }
    match hide {
        Hide::Artifact => segs.first().is_some_and(|s| *s == ".github"),
        Hide::Mirror => segs.first().is_some_and(|s| *s == "_mirror"),
        Hide::Jekyll => {
            segs.iter().any(|s| (s.starts_with('_') || s.starts_with('.')) && *s != ".well-known")
                || segs.first().is_some_and(|s| matches!(*s, "node_modules" | "vendor" | "Gemfile" | "Gemfile.lock"))
        }
    }
}

/// Whether `rel` is a file under `root`, matching the case of every name (as
/// Pages does: on Windows and macOS the disk alone would match any case).
pub fn is_file(root: &Path, rel: &str) -> bool {
    let p = root.join(rel);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_file() => exact_case(root, rel),
        _ => false,
    }
}

fn is_dir(root: &Path, rel: &str) -> bool {
    let p = root.join(rel);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_dir() => exact_case(root, rel),
        _ => false,
    }
}

fn exact_case(root: &Path, rel: &str) -> bool {
    if !cfg!(any(windows, target_os = "macos")) {
        return true;
    }
    let mut dir = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        let Ok(rd) = std::fs::read_dir(&dir) else { return false };
        if !rd.flatten().any(|e| e.file_name().to_string_lossy() == seg) {
            return false;
        }
        dir.push(seg);
    }
    true
}

/// Read a `_redirects` file (Netlify's format, which Cloudflare Pages shares):
/// `from to [status]` per line, `*` at the end of `from` and `:splat` in `to`,
/// `:name` placeholders, # comments.
pub fn parse_redirects(text: &str) -> Vec<Redirect> {
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next().unwrap_or("").trim();
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() < 2 || !parts[0].starts_with('/') {
                return None;
            }
            let status = parts.get(2).and_then(|s| s.trim_end_matches('!').parse().ok()).unwrap_or(301);
            Some(Redirect { from: parts[0].to_string(), to: parts[1].to_string(), status })
        })
        .collect()
}

/// The first rule matching `path` ("/x/y", after the base): where it leads and its status.
pub fn redirect_for(rules: &[Redirect], path: &str) -> Option<(String, u16)> {
    for r in rules {
        if let Some(vars) = match_pattern(&r.from, path) {
            let mut to = r.to.clone();
            for (k, v) in &vars {
                to = to.replace(&format!(":{k}"), v);
            }
            return Some((to, r.status));
        }
    }
    None
}

/// Match a path against a pattern with `:name` segments and a final `*`.
fn match_pattern(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    let mut vars = vec![];
    let pat: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    for (i, p) in pat.iter().enumerate() {
        if *p == "*" {
            vars.push(("splat".to_string(), segs.get(i..).map(|s| s.join("/")).unwrap_or_default()));
            return Some(vars);
        }
        let s = segs.get(i)?;
        if let Some(name) = p.strip_prefix(':') {
            vars.push((name.to_string(), s.to_string()));
        } else if p != s {
            return None;
        }
    }
    (pat.len() == segs.len()).then_some(vars)
}

/// Read a `_headers` file: a path pattern on its own line, then indented `Name: value` lines.
pub fn parse_headers(text: &str) -> Vec<HeaderRule> {
    let mut out: Vec<HeaderRule> = vec![];
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            out.push(HeaderRule { pattern: line.trim().to_string(), headers: vec![] });
        } else if let (Some(rule), Some((k, v))) = (out.last_mut(), line.trim().split_once(':')) {
            rule.headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// The headers `_headers` gives a path.
pub fn headers_for(rules: &[HeaderRule], path: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    for r in rules {
        if glob(&r.pattern, path) {
            out.extend(r.headers.iter().cloned());
        }
    }
    out
}

fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s || match_pattern(pattern, s).is_some();
    }
    let mut pos = 0;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            if !s.starts_with(p) {
                return false;
            }
            pos = p.len();
        } else if i == parts.len() - 1 {
            return s.len() >= pos + p.len() && s.ends_with(p);
        } else {
            match s[pos..].find(p) {
                Some(k) => pos += k + p.len(),
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("reposync-router-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        for (p, t) in [
            ("index.html", "home"),
            ("about.html", "about"),
            ("docs/index.html", "docs"),
            ("Images/Logo.png", "png"),
            ("textures/body.jpg", "jpg"),
            ("404.html", "missing"),
            ("_drafts/x.html", "draft"),
            (".github/workflows/p.yml", "wf"),
            ("notes/index.md", "# Notes"),
            ("guide.md", "# Guide"),
            ("plain/readme.txt", "no index"),
        ] {
            let f = d.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        }
        d
    }

    #[test]
    fn pages_rules() {
        let d = site_dir();
        let s = Site { root: &d, base: "/vectorclad/", hide: Hide::Artifact, spa: false, markdown: false, redirects: &[] };
        let f = |p: &str| decide(&s, p, "", true);
        assert_eq!(f("/"), Decision::Redirect("/vectorclad/".into(), 302));
        assert_eq!(decide(&s, "/vectorclad", "a=1", true), Decision::Redirect("/vectorclad/?a=1".into(), 301));
        assert_eq!(f("/vectorclad/"), Decision::File(d.join("index.html")));
        assert_eq!(f("/other/x"), Decision::NotFound(None));
        assert_eq!(f("/vectorclad/textures/body.jpg"), Decision::File(d.join("textures/body.jpg")));
        assert_eq!(decide(&s, "/vectorclad/docs", "q=2", true), Decision::Redirect("/vectorclad/docs/?q=2".into(), 301));
        assert_eq!(f("/vectorclad/docs/"), Decision::File(d.join("docs/index.html")));
        assert_eq!(f("/vectorclad/about"), Decision::File(d.join("about.html")));
        assert_eq!(f("/vectorclad/nothing"), Decision::NotFound(Some(d.join("404.html"))));
        assert_eq!(f("/vectorclad/plain/"), Decision::NotFound(Some(d.join("404.html"))));
        assert_eq!(f("/vectorclad/.github/workflows/p.yml"), Decision::NotFound(Some(d.join("404.html"))));
        assert_eq!(f("/vectorclad/_drafts/x.html"), Decision::File(d.join("_drafts/x.html")), "an Actions deployment keeps _ folders");
        assert_eq!(f("/vectorclad/Images/Logo.png"), Decision::File(d.join("Images/Logo.png")));
        if cfg!(any(windows, target_os = "macos")) || !d.join("images/logo.png").exists() {
            assert!(matches!(f("/vectorclad/images/logo.png"), Decision::NotFound(_)), "names match case for case");
        }
        // the SPA fallback, only when asked, only for pages
        let spa = Site { spa: true, ..s };
        assert_eq!(decide(&spa, "/vectorclad/app/route", "", true), Decision::File(d.join("index.html")));
        assert!(matches!(decide(&spa, "/vectorclad/app/route", "", false), Decision::NotFound(_)));
        assert!(matches!(decide(&spa, "/vectorclad/missing.png", "", true), Decision::NotFound(_)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn jekyll_branch_rules() {
        let d = site_dir();
        let s = Site { root: &d, base: "/", hide: Hide::Jekyll, spa: false, markdown: true, redirects: &[] };
        let f = |p: &str| decide(&s, p, "", true);
        assert!(matches!(f("/_drafts/x.html"), Decision::NotFound(_)), "Jekyll leaves out _ folders");
        assert_eq!(f("/notes/"), Decision::Markdown(d.join("notes/index.md")));
        assert_eq!(f("/guide.html"), Decision::Markdown(d.join("guide.md")));
        assert_eq!(f("/guide"), Decision::Markdown(d.join("guide.md")));
        assert_eq!(f("/"), Decision::File(d.join("index.html")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unsafe_paths_are_refused() {
        for p in ["..%2fsecret", "a/../../b", "%5cwin", "a.", "a ", "CON", "con.txt", "x::$DATA", "a%00b", "a//b", "%zz", "lpt1/x"] {
            assert_eq!(clean(p), None, "{p}");
        }
        assert_eq!(clean("a%20b/c.js").as_deref(), Some("a b/c.js"));
        assert_eq!(clean("").as_deref(), Some(""));
        assert_eq!(clean("dir/").as_deref(), Some("dir/"));
        let d = site_dir();
        let s = Site { root: &d, base: "/", hide: Hide::Artifact, spa: false, markdown: false, redirects: &[] };
        assert_eq!(decide(&s, "/..%2f..%2fetc%2fpasswd", "", true), Decision::Bad);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn redirects_and_headers_files() {
        let rules = parse_redirects("# comment\n/old /new 301\n/blog/* /posts/:splat 302\n/u/:id /users.html?id=:id 200\n/gone /404 404\n/ext https://example.com\n");
        assert_eq!(redirect_for(&rules, "/old"), Some(("/new".into(), 301)));
        assert_eq!(redirect_for(&rules, "/blog/2024/a"), Some(("/posts/2024/a".into(), 302)));
        assert_eq!(redirect_for(&rules, "/u/7"), Some(("/users.html?id=7".into(), 200)));
        assert_eq!(redirect_for(&rules, "/ext"), Some(("https://example.com".into(), 301)));
        assert_eq!(redirect_for(&rules, "/nothing"), None);
        let d = site_dir();
        let s = Site { root: &d, base: "/site/", hide: Hide::Artifact, spa: false, markdown: false, redirects: &rules };
        assert_eq!(decide(&s, "/site/old", "", true), Decision::Redirect("/site/new".into(), 301));
        let rw = parse_redirects("/home /about.html 200");
        let s = Site { redirects: &rw, ..s };
        assert_eq!(decide(&s, "/site/home", "", true), Decision::File(d.join("about.html")));
        let h = parse_headers("/*\n  X-Frame-Options: DENY\n/assets/*\n  Cache-Control: max-age=60\n");
        assert_eq!(headers_for(&h, "/assets/a.js").len(), 2);
        assert_eq!(headers_for(&h, "/index.html"), vec![("X-Frame-Options".to_string(), "DENY".to_string())]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
