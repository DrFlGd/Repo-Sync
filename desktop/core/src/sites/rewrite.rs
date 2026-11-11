//! Pointing a page at the saved copies of files from other servers, as the
//! page is served: `https://cdn.jsdelivr.net/…` becomes
//! `/__reposync/v/cdn.jsdelivr.net/…`, in HTML (attributes, inline scripts,
//! import maps), scripts and stylesheets. The captured files themselves are
//! never changed (docs/PLAN.md, "Working offline").

use super::scan;
use regex::Regex;
use std::sync::OnceLock;

/// The route saved files are served from.
pub const ROUTE: &str = "/__reposync/v/";

/// `text` with every address on one of `hosts` (http, https or protocol-relative)
/// pointed at the saved copies, and whether anything changed.
pub fn swap_hosts(text: &str, hosts: &[String]) -> (String, bool) {
    swap_hosts_to(text, hosts, ROUTE)
}

/// As [`swap_hosts`], with another start for the new addresses (an export uses a
/// relative "../vendor/").
pub fn swap_hosts_to(text: &str, hosts: &[String], to: &str) -> (String, bool) {
    if hosts.is_empty() || !text.contains("//") {
        return (text.to_string(), false);
    }
    let alts: Vec<String> = hosts.iter().map(|h| regex::escape(h)).collect();
    let Ok(re) = Regex::new(&format!(r"(?i)(?:https?:)?//({})", alts.join("|"))) else { return (text.to_string(), false) };
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    for c in re.captures_iter(text) {
        let m = c.get(0).unwrap();
        let host = c.get(1).unwrap().as_str().to_ascii_lowercase();
        // the host must end there ("cdn.example.com" isn't "cdn.example.com.evil")
        let next = text[m.end()..].chars().next();
        if next.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '@' || ch == ':') {
            continue;
        }
        // "//host" alone only where an address starts (not inside a path or a comment)
        if !m.as_str().to_ascii_lowercase().starts_with("http") {
            let before = text[..m.start()].chars().last();
            if !matches!(before, None | Some('"' | '\'' | '(' | '=' | ' ' | '`' | ',' | '\n')) {
                continue;
            }
        }
        out.push_str(&text[last..m.start()]);
        out.push_str(to);
        out.push_str(&host);
        last = m.end();
        changed = true;
    }
    out.push_str(&text[last..]);
    (out, changed)
}

static INTEGRITY: OnceLock<Regex> = OnceLock::new();

/// HTML without `integrity="…"` checks (a saved file served with its addresses
/// changed no longer matches the hash the page was written with).
pub fn strip_integrity(html: &str) -> String {
    let re = INTEGRITY.get_or_init(|| Regex::new(r#"(?i)\sintegrity\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#).unwrap());
    re.replace_all(html, "").into_owned()
}

static CSS_ROOT_URL: OnceLock<Regex> = OnceLock::new();

/// A saved script or stylesheet from `host` with its root-relative addresses
/// ("/npm/x@1/+esm", "/v135/react.mjs") pointed at the saved copies on that
/// host: in the page they would mean the hosted site's own root.
pub fn root_relative_to_route(text: &str, kind: &str, host: &str, to: &str) -> String {
    let prefix = format!("{to}{host}");
    match kind {
        "js" => {
            let mut out = String::with_capacity(text.len());
            let mut last = 0;
            for (u, _, range) in scan::js_literals(text) {
                if u.starts_with('/') && !u.starts_with("//") && !u.starts_with(to) {
                    out.push_str(&text[last..range.start]);
                    out.push_str(&prefix);
                    last = range.start;
                }
            }
            out.push_str(&text[last..]);
            out
        }
        "css" => {
            let re = CSS_ROOT_URL.get_or_init(|| Regex::new(r#"(?i)(url\(\s*['"]?|@import\s+['"])/([^/'")\s][^'")\s]*)"#).unwrap());
            let already = to.trim_start_matches('/');
            re.replace_all(text, |c: &regex::Captures| if !already.is_empty() && c[2].starts_with(already) { c[0].to_string() } else { format!("{}{prefix}/{}", &c[1], &c[2]) }).into_owned()
        }
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_on_saved_hosts_are_swapped() {
        let hosts = vec!["cdn.jsdelivr.net".to_string(), "fonts.googleapis.com".to_string()];
        let html = r#"<link rel="preconnect" href="https://fonts.googleapis.com"><link href="https://fonts.googleapis.com/css2?family=A&amp;b=1" rel=stylesheet>
<script type="importmap">{"imports":{"three":"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js"}}</script>
<script src="//cdn.jsdelivr.net/npm/x@1/x.js" integrity="sha384-abc" crossorigin></script>
<a href="https://cdn.jsdelivr.net.evil.com/x">no</a> <p>see path//cdn.jsdelivr.net/x</p> <a href="https://unpkg.com/a">kept</a>"#;
        let (out, changed) = swap_hosts(html, &hosts);
        assert!(changed);
        assert!(out.contains(r#"href="/__reposync/v/fonts.googleapis.com""#));
        assert!(out.contains("/__reposync/v/fonts.googleapis.com/css2?family=A&amp;b=1"));
        assert!(out.contains(r#""three":"/__reposync/v/cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js""#));
        assert!(out.contains(r#"src="/__reposync/v/cdn.jsdelivr.net/npm/x@1/x.js""#));
        assert!(out.contains("https://cdn.jsdelivr.net.evil.com/x") && out.contains("path//cdn.jsdelivr.net/x") && out.contains("https://unpkg.com/a"));
        let clean = strip_integrity(&out);
        assert!(!clean.contains("integrity") && clean.contains("crossorigin"));
        assert_eq!(swap_hosts("nothing", &hosts), ("nothing".to_string(), false));
    }

    #[test]
    fn root_relative_addresses_in_saved_files() {
        let js = r#"import "/npm/three@0.170.0/+esm"; import a from "./a.js"; export * from "/v135/react@18/es2022/react.mjs"; fetch("//x.com/y");"#;
        let out = root_relative_to_route(js, "js", "esm.sh", ROUTE);
        assert!(out.contains(r#"import "/__reposync/v/esm.sh/npm/three@0.170.0/+esm""#), "{out}");
        assert!(out.contains(r#"from "/__reposync/v/esm.sh/v135/react@18/es2022/react.mjs""#));
        assert!(out.contains(r#"from "./a.js""#) && out.contains(r#"fetch("//x.com/y")"#));
        let css = "a{background:url(/img/a.png)} @import '/b.css'; b{background:url(c.png)}";
        let out = root_relative_to_route(css, "css", "cdnjs.cloudflare.com", ROUTE);
        assert!(out.contains("url(/__reposync/v/cdnjs.cloudflare.com/img/a.png)") && out.contains("@import '/__reposync/v/cdnjs.cloudflare.com/b.css'") && out.contains("url(c.png)"), "{out}");
    }
}
