//! Finding the addresses a page, script or stylesheet refers to: the rules
//! WebMirror (DrFlGd/WebMirror, webmirror/core.py) uses to save a site, ported:
//! HTML attributes, import maps, `import`/`export … from`, `import()`,
//! `fetch()`, `new URL()`, workers, `importScripts`, CSS `url()` and `@import`,
//! and the chunks a webpack runtime loads later.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use url::Url;

/// What a reference is for: a page to follow, or a file the page needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// `<a href>`, `<iframe src>`, `<area href>`: another page.
    Page,
    /// A script or module (`<script src>`, `modulepreload`, `import`).
    Script,
    Style,
    /// A picture, font, sound, video, manifest or anything else a page loads.
    Asset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ref {
    pub url: String,
    pub kind: RefKind,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a valid pattern"))
}

static TAG: OnceLock<Regex> = OnceLock::new();
static ATTR: OnceLock<Regex> = OnceLock::new();
static SCRIPT: OnceLock<Regex> = OnceLock::new();
static STYLE: OnceLock<Regex> = OnceLock::new();
static CSS_URL: OnceLock<Regex> = OnceLock::new();
static CSS_IMPORT: OnceLock<Regex> = OnceLock::new();
static JS_LITERAL: OnceLock<Regex> = OnceLock::new();
static IMPORT_SCRIPTS: OnceLock<Regex> = OnceLock::new();
static QUOTED: OnceLock<Regex> = OnceLock::new();
static ABS_URL: OnceLock<Regex> = OnceLock::new();
static WEBPACK_PUBLIC_PATH: OnceLock<Regex> = OnceLock::new();
static WEBPACK_CHUNK_PATH: OnceLock<Regex> = OnceLock::new();
static WEBPACK_CHUNK_MAP: OnceLock<Regex> = OnceLock::new();
static MAP_PAIR: OnceLock<Regex> = OnceLock::new();
static NAME_TERNARY: OnceLock<Regex> = OnceLock::new();
static NAME_PAIR: OnceLock<Regex> = OnceLock::new();
static TITLE: OnceLock<Regex> = OnceLock::new();
static COMMENT: OnceLock<Regex> = OnceLock::new();
static BLOCK_END: OnceLock<Regex> = OnceLock::new();
static ANY_TAG: OnceLock<Regex> = OnceLock::new();

/// Tags with their attribute text: `<img src="x" alt=y>`.
pub fn tags(html: &str) -> impl Iterator<Item = (String, &str, std::ops::Range<usize>)> {
    re(&TAG, r"(?is)<([a-z][a-z0-9-]*)(\s[^>]*)?>").captures_iter(html).map(|c| {
        let all = c.get(0).unwrap();
        (c[1].to_ascii_lowercase(), c.get(2).map(|m| m.as_str()).unwrap_or(""), all.range())
    })
}

/// A tag's attributes, names lower-cased, values with `&amp;` and friends decoded.
pub fn attrs(text: &str) -> Vec<(String, String)> {
    re(&ATTR, r#"(?s)([^\s=/>"']+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>"']+)))?"#)
        .captures_iter(text)
        .map(|c| {
            let v = c.get(2).or(c.get(3)).or(c.get(4)).map(|m| m.as_str()).unwrap_or("");
            (c[1].to_ascii_lowercase(), decode_entities(v))
        })
        .collect()
}

pub fn attr<'a>(list: &'a [(String, String)], name: &str) -> Option<&'a str> {
    list.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// The few entities addresses use.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&").replace("&#38;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// Inline scripts: (the `<script …>` attributes, the code).
pub fn scripts(html: &str) -> Vec<(Vec<(String, String)>, String)> {
    re(&SCRIPT, r"(?is)<script(\s[^>]*)?>(.*?)</script\s*>")
        .captures_iter(html)
        .map(|c| (attrs(c.get(1).map(|m| m.as_str()).unwrap_or("")), c[2].to_string()))
        .collect()
}

fn styles(html: &str) -> Vec<String> {
    re(&STYLE, r"(?is)<style(?:\s[^>]*)?>(.*?)</style\s*>").captures_iter(html).map(|c| c[1].to_string()).collect()
}

/// `<base href>`, if the page has one.
pub fn base_href(html: &str) -> Option<String> {
    tags(html).find(|(t, _, _)| t == "base").and_then(|(_, a, _)| attr(&attrs(a), "href").map(String::from))
}

/// The page's title.
pub fn title(html: &str) -> String {
    re(&TITLE, r"(?is)<title[^>]*>(.*?)</title\s*>").captures(html).map(|c| decode_entities(c[1].trim()).split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// The words a page shows, for search (scripts, styles and tags left out).
pub fn text(html: &str) -> String {
    let no_scripts = re(&SCRIPT, r"(?is)<script(\s[^>]*)?>(.*?)</script\s*>").replace_all(html, " ");
    let no_styles = re(&STYLE, r"(?is)<style(?:\s[^>]*)?>(.*?)</style\s*>").replace_all(&no_scripts, " ");
    let no_comments = re(&COMMENT, r"(?s)<!--.*?-->").replace_all(&no_styles, " ");
    // a block's end starts a new line, so changes can be shown line by line
    let blocks = re(&BLOCK_END, r"(?i)</(?:p|div|h[1-6]|li|tr|td|th|title|section|article|header|footer|nav|main|aside|blockquote|pre|dt|dd|figcaption|table|ul|ol)\s*>|<br\s*/?>").replace_all(&no_comments, "\n");
    let plain = re(&ANY_TAG, r"(?s)</?[a-zA-Z!][^>]*>").replace_all(&blocks, " ");
    let decoded = decode_entities(&plain).replace("&nbsp;", " ");
    decoded.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

const SKIP: [&str; 8] = ["data:", "blob:", "javascript:", "mailto:", "tel:", "about:", "#", "{{"];

fn skip(u: &str) -> bool {
    let u = u.trim();
    u.is_empty() || SKIP.iter().any(|p| u.to_ascii_lowercase().starts_with(p))
}

/// Every address an HTML page refers to, as written (not yet resolved).
pub fn html_refs(html: &str) -> Vec<Ref> {
    let mut out = vec![];
    let mut push = |url: &str, kind| {
        if !skip(url) {
            out.push(Ref { url: url.trim().to_string(), kind });
        }
    };
    for (tag, a, _) in tags(html) {
        let a = attrs(a);
        match tag.as_str() {
            "a" | "area" => {
                if let Some(h) = attr(&a, "href") {
                    push(h, RefKind::Page);
                }
            }
            "iframe" | "frame" => {
                if let Some(h) = attr(&a, "src") {
                    push(h, RefKind::Page);
                }
            }
            "script" => {
                if let Some(h) = attr(&a, "src") {
                    push(h, RefKind::Script);
                }
            }
            "link" => {
                let rel = attr(&a, "rel").unwrap_or("").to_ascii_lowercase();
                if let Some(h) = attr(&a, "href") {
                    if rel.split_whitespace().any(|r| r == "stylesheet") {
                        push(h, RefKind::Style);
                    } else if rel.split_whitespace().any(|r| r == "modulepreload") || (rel.contains("preload") && attr(&a, "as") == Some("script")) {
                        push(h, RefKind::Script);
                    } else if rel.split_whitespace().any(|r| matches!(r, "icon" | "shortcut" | "apple-touch-icon" | "manifest" | "preload" | "prefetch" | "mask-icon")) {
                        push(h, RefKind::Asset);
                    }
                }
            }
            "img" | "source" | "video" | "audio" | "track" | "embed" | "input" => {
                if let Some(h) = attr(&a, "src") {
                    push(h, RefKind::Asset);
                }
            }
            "object" => {
                if let Some(h) = attr(&a, "data") {
                    push(h, RefKind::Asset);
                }
            }
            _ => {}
        }
        if let Some(p) = attr(&a, "poster") {
            push(p, RefKind::Asset);
        }
        for key in ["srcset", "imagesrcset"] {
            if let Some(set) = attr(&a, key) {
                for item in set.split(',') {
                    if let Some(u) = item.split_whitespace().next() {
                        push(u, RefKind::Asset);
                    }
                }
            }
        }
        if let Some(s) = attr(&a, "style") {
            for u in css_refs(s) {
                push(&u.url, u.kind);
            }
        }
    }
    for css in styles(html) {
        for u in css_refs(&css) {
            out.push(u);
        }
    }
    for (a, code) in scripts(html) {
        let t = attr(&a, "type").unwrap_or("").to_ascii_lowercase();
        if t == "importmap" {
            for (_, v) in import_map(&code) {
                if !v.ends_with('/') {
                    out.push(Ref { url: v, kind: RefKind::Script });
                }
            }
        } else if attr(&a, "src").is_none() && matches!(t.as_str(), "" | "module" | "text/javascript" | "application/javascript") {
            out.extend(js_refs(&code));
        }
    }
    out
}

/// The entries of an import map: (specifier, address).
pub fn import_map(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return vec![] };
    let mut out = vec![];
    let mut add = |m: &Value| {
        if let Some(m) = m.as_object() {
            for (k, v) in m {
                if let Some(s) = v.as_str() {
                    out.push((k.clone(), s.to_string()));
                }
            }
        }
    };
    add(&v["imports"]);
    if let Some(scopes) = v["scopes"].as_object() {
        for m in scopes.values() {
            add(m);
        }
    }
    out
}

/// Every import map in a page, merged (the first entry for a specifier wins).
pub fn page_import_map(html: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for (a, code) in scripts(html) {
        if attr(&a, "type").map(|t| t.eq_ignore_ascii_case("importmap")).unwrap_or(false) {
            for (k, v) in import_map(&code) {
                if !out.iter().any(|(x, _)| *x == k) {
                    out.push((k, v));
                }
            }
        }
    }
    out
}

/// What a bare specifier ("three", "three/addons/x.js") maps to in an import map.
pub fn resolve_specifier(map: &[(String, String)], spec: &str) -> Option<String> {
    if let Some((_, v)) = map.iter().find(|(k, _)| k == spec) {
        return Some(v.clone());
    }
    map.iter().filter(|(k, _)| k.ends_with('/') && spec.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()).map(|(k, v)| format!("{v}{}", &spec[k.len()..]))
}

/// Whether a module specifier is bare ("three"), not an address ("./x.js", "/x", "https://…").
pub fn is_bare(spec: &str) -> bool {
    !(spec.starts_with('.') || spec.starts_with('/') || spec.contains("://") || spec.starts_with("data:") || spec.starts_with("blob:"))
}

fn js_literal() -> &'static Regex {
    re(
        &JS_LITERAL,
        r#"(?m)(?P<prefix>\b(?:import|export)\s+(?:[^\n;]{0,200}?\s+from\s*)?|\bimport\s*\(\s*|\bnew\s+URL\s*\(\s*|\bfetch\s*\(\s*|\bnew\s+(?:Worker|SharedWorker)\s*\(\s*)(?P<q>['"])(?P<url>[^'"\n]+)['"]"#,
    )
}

/// The addresses in a script, with the byte range of each in `js`. Literals that
/// are only the start of an address built at run time (`fetch("/x/" + name)`)
/// are left out.
pub fn js_literals(js: &str) -> Vec<(String, RefKind, std::ops::Range<usize>)> {
    let mut out = vec![];
    for c in js_literal().captures_iter(js) {
        let (Some(u), Some(q)) = (c.name("url"), c.name("q")) else { continue };
        // the quote must close the literal it opened
        let close = u.end();
        if js.as_bytes().get(close).copied() != Some(q.as_str().as_bytes()[0]) {
            continue;
        }
        if js[close + 1..].trim_start().starts_with('+') {
            continue;
        }
        let prefix = c.name("prefix").map(|m| m.as_str()).unwrap_or("");
        let kind = if prefix.trim_start().starts_with("fetch") || prefix.contains("URL") { RefKind::Asset } else { RefKind::Script };
        out.push((u.as_str().to_string(), kind, u.range()));
    }
    for c in re(&IMPORT_SCRIPTS, r"\bimportScripts\s*\(([^)]*)\)").captures_iter(js) {
        let args = c.get(1).unwrap();
        for q in re(&QUOTED, r#"['"]([^'"\n]+)['"]"#).captures_iter(args.as_str()) {
            let m = q.get(1).unwrap();
            out.push((m.as_str().to_string(), RefKind::Script, args.start() + m.start()..args.start() + m.end()));
        }
    }
    out
}

/// Every address a script refers to (bare specifiers included, for the import map).
pub fn js_refs(js: &str) -> Vec<Ref> {
    js_literals(js).into_iter().filter(|(u, _, _)| !skip(u)).map(|(url, kind, _)| Ref { url, kind }).collect()
}

/// Every address a stylesheet refers to.
pub fn css_refs(css: &str) -> Vec<Ref> {
    let mut out = vec![];
    for c in re(&CSS_IMPORT, r#"(?i)@import\s+(?:url\(\s*)?(['"])([^'"]+)['"]"#).captures_iter(css) {
        if !skip(&c[2]) {
            out.push(Ref { url: c[2].trim().to_string(), kind: RefKind::Style });
        }
    }
    for c in re(&CSS_URL, r#"(?i)url\(\s*(?:"([^"]*)"|'([^']*)'|([^'")]*))\s*\)"#).captures_iter(css) {
        let u = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
        if !skip(u) {
            out.push(Ref { url: u.trim().to_string(), kind: RefKind::Asset });
        }
    }
    out
}

/// Every chunk a webpack runtime can load later, as addresses (WebMirror's
/// `_webpack_chunks`): chunks load only when the page needs them, so a visit
/// misses some, and offline a missing one breaks the app.
pub fn webpack_chunks(js: &str, base: &Url) -> Vec<String> {
    let root = if let Some(c) = re(&WEBPACK_PUBLIC_PATH, r#"\b\w+\.p\s*=\s*['"]([^'"]*)['"]"#).captures(js) {
        match base.join(&c[1]) {
            Ok(u) => u,
            Err(_) => return vec![],
        }
    } else if let Some((before, _)) = base.as_str().split_once("/_next/static/") {
        match Url::parse(&format!("{before}/_next/")) {
            Ok(u) => u,
            Err(_) => return vec![],
        }
    } else {
        return vec![];
    };
    let mut paths: Vec<String> = re(&WEBPACK_CHUNK_PATH, r#"['"](static/(?:chunks|css)/[^'"+]+\.(?:js|css))['"]"#).captures_iter(js).map(|c| c[1].to_string()).collect();
    let map_re = re(
        &WEBPACK_CHUNK_MAP,
        r#"['"](?P<folder>static/(?:chunks|css)/)['"]\s*\+\s*(?P<name>\([^;]*?\)|\w+)\s*\+\s*['"](?P<sep>[.-])['"]\s*\+\s*\(?\s*(?P<map>\{[^{}]*\})\s*\)?\s*\[\w+\]\s*\+\s*['"](?P<ext>\.(?:js|css))['"]"#,
    );
    for c in map_re.captures_iter(js) {
        let hashes: Vec<(String, String)> = re(&MAP_PAIR, r#"['"]?([\w-]+)['"]?\s*:\s*['"]([0-9a-f]{6,})['"]"#).captures_iter(&c["map"]).map(|m| (m[1].to_string(), m[2].to_string())).collect();
        let mut names: Vec<(String, String)> = re(&NAME_TERNARY, r#"(\d+)\s*===\s*\w+\s*\?\s*['"]([^'"]+)['"]"#).captures_iter(&c["name"]).map(|m| (m[1].to_string(), m[2].to_string())).collect();
        names.extend(re(&NAME_PAIR, r#"['"]?(\d+)['"]?\s*:\s*['"]([^'"]+)['"]"#).captures_iter(&c["name"]).map(|m| (m[1].to_string(), m[2].to_string())));
        for (chunk, digest) in hashes {
            let name = names.iter().find(|(k, _)| *k == chunk).map(|(_, v)| v.clone()).unwrap_or(chunk);
            paths.push(format!("{}{name}{}{digest}{}", &c["folder"], &c["sep"], &c["ext"]));
        }
    }
    paths.sort();
    paths.dedup();
    paths.into_iter().filter_map(|p| root.join(&p).ok().map(|u| u.to_string())).collect()
}

/// Absolute addresses written anywhere in a text ("https://host/path", "//host/path"),
/// for files fetched from other servers by code the scanner can't follow.
pub fn absolute_urls(text: &str) -> Vec<String> {
    re(&ABS_URL, r#"(?i)(?:https?:)?//([a-z0-9][a-z0-9.-]*\.[a-z]{2,})(/[^\s"'<>()\\`{}$|^\[\]]*)?"#)
        .captures_iter(text)
        .filter_map(|c| {
            let m = c.get(0).unwrap();
            // "//" in the middle of a word or a path isn't an address
            if !m.as_str().to_ascii_lowercase().starts_with("http") {
                let before = text[..m.start()].chars().last();
                if !matches!(before, None | Some('"' | '\'' | '(' | '=' | ' ' | '`')) {
                    return None;
                }
            }
            // the start of an address built at run time (`https://x/${name}`)
            if text[m.end()..].starts_with(['$', '{', '+']) || text[m.end()..].trim_start().starts_with('+') {
                return None;
            }
            let path = c.get(2).map(|p| p.as_str()).unwrap_or("/");
            let path = path.trim_end_matches(['.', ',', ';', ':']);
            Some(format!("https://{}{}", c[1].to_ascii_lowercase(), if path.is_empty() { "/" } else { path }))
        })
        .collect()
}

/// `reference` resolved against `base`, without its #fragment; None for addresses
/// that aren't http(s).
pub fn resolve(base: &Url, reference: &str) -> Option<Url> {
    let r = reference.trim();
    if skip(r) {
        return None;
    }
    let mut u = base.join(r).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    u.set_fragment(None);
    Some(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_references() {
        let html = r#"<!doctype html><html><head><title> Hello &amp; welcome </title>
<link rel="stylesheet" href="style.css"><link rel=icon href="/fav.ico"><link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=A&amp;display=swap">
<script type="importmap">{"imports":{"three":"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js","three/addons/":"https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/"}}</script>
<script type="module">import { x } from './js/main.js'; import('./lazy.js');</script>
<style>body{background:url('bg.png')}</style></head>
<body><a href="about/">About</a><a href="mailto:x@y">mail</a><img src="a.png" srcset="a@2x.png 2x, a@3x.png 3x"><div style="background:url(d.png)"></div>
<video poster="p.jpg"><source src="v.mp4"></video></body></html>"#;
        let refs = html_refs(html);
        let has = |u: &str, k| refs.iter().any(|r| r.url == u && r.kind == k);
        assert!(has("style.css", RefKind::Style));
        assert!(has("/fav.ico", RefKind::Asset));
        assert!(!refs.iter().any(|r| r.url == "https://fonts.googleapis.com"), "preconnect needs nothing");
        assert!(has("https://fonts.googleapis.com/css2?family=A&display=swap", RefKind::Style));
        assert!(has("https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js", RefKind::Script));
        assert!(has("./js/main.js", RefKind::Script) && has("./lazy.js", RefKind::Script));
        assert!(has("bg.png", RefKind::Asset) && has("d.png", RefKind::Asset));
        assert!(has("about/", RefKind::Page) && !refs.iter().any(|r| r.url.starts_with("mailto")));
        assert!(has("a@2x.png", RefKind::Asset) && has("a@3x.png", RefKind::Asset) && has("p.jpg", RefKind::Asset) && has("v.mp4", RefKind::Asset));
        assert_eq!(title(html), "Hello & welcome");
        let map = page_import_map(html);
        assert_eq!(resolve_specifier(&map, "three/addons/loaders/GLTFLoader.js").as_deref(), Some("https://cdn.jsdelivr.net/npm/three@0.170.0/examples/jsm/loaders/GLTFLoader.js"));
        assert_eq!(resolve_specifier(&map, "three").as_deref(), Some("https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js"));
        assert_eq!(resolve_specifier(&map, "react"), None);
        assert!(text(html).contains("About") && !text(html).contains("import"));
    }

    #[test]
    fn script_references() {
        let js = r#"import a from "./a.js"; export * from 'https://cdn.example/x.js';
import { b } from "three"; const c = await import('../c.js'); fetch("data/level.json");
fetch("/tools/" + name); new Worker(new URL('./w.js', import.meta.url)); importScripts('s1.js', "s2.js");
const t = `${base}/x.js`;"#;
        let refs: Vec<String> = js_refs(js).into_iter().map(|r| r.url).collect();
        for u in ["./a.js", "https://cdn.example/x.js", "three", "../c.js", "data/level.json", "./w.js", "s1.js", "s2.js"] {
            assert!(refs.contains(&u.to_string()), "{u} in {refs:?}");
        }
        assert!(!refs.iter().any(|u| u.starts_with("/tools/")), "an address built at run time is left alone");
        assert!(is_bare("three") && !is_bare("./a.js") && !is_bare("/x") && !is_bare("https://x/y"));
    }

    #[test]
    fn stylesheet_references() {
        let css = r#"@import url("base.css"); @import 'more.css'; a{background:url(i.png)} b{src:url("f.woff2") format("woff2")} c{x:url(data:image/png;base64,AA)}"#;
        let refs: Vec<String> = css_refs(css).into_iter().map(|r| r.url).collect();
        assert_eq!(refs.iter().filter(|u| *u == "base.css").count(), 2, "{refs:?}"); // as an import and as a url()
        assert!(refs.contains(&"more.css".into()) && refs.contains(&"i.png".into()) && refs.contains(&"f.woff2".into()));
        assert!(!refs.iter().any(|u| u.starts_with("data:")));
    }

    #[test]
    fn webpack_chunks_loaded_later_are_found() {
        // WebMirror's test_webpack_chunks_loaded_later_are_saved, ported
        let js = r#"r.p="/_next/";r.u=function(e){return"static/chunks/"+e+"."+{7701:"6846ef15632e87c4",42:"aaaaaaaa"}[e]+".js"};x="static/css/app.123abc.css""#;
        let base = Url::parse("https://example.com/_next/static/chunks/webpack.js").unwrap();
        let c = webpack_chunks(js, &base);
        assert!(c.contains(&"https://example.com/_next/static/chunks/7701.6846ef15632e87c4.js".to_string()), "{c:?}");
        assert!(c.contains(&"https://example.com/_next/static/chunks/42.aaaaaaaa.js".to_string()));
        assert!(c.contains(&"https://example.com/_next/static/css/app.123abc.css".to_string()));
        assert!(webpack_chunks("nothing here", &Url::parse("https://e.com/app.js").unwrap()).is_empty());
    }

    #[test]
    fn absolute_addresses() {
        let t = r#"src="https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js" x='//unpkg.com/a@1/b.js' path//not.this/ `https://cdn.jsdelivr.net/gh/o/r@${v}/x`"#;
        let u = absolute_urls(t);
        assert!(u.contains(&"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.min.js".to_string()));
        assert!(u.contains(&"https://unpkg.com/a@1/b.js".to_string()));
        assert!(!u.iter().any(|x| x.contains("not.this")));
        assert!(!u.iter().any(|x| x.contains("/gh/o/")), "an address built at run time is left alone: {u:?}");
        let base = Url::parse("https://a.com/x/y.html").unwrap();
        assert_eq!(resolve(&base, "../z.png#top").unwrap().as_str(), "https://a.com/z.png");
        assert!(resolve(&base, "javascript:void(0)").is_none());
    }
}
