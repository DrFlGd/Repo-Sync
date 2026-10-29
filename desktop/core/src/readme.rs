//! A captured readme as HTML for the project's page. Markdown is formatted (with
//! the HTML a readme may hold, cleaned by ammonia so nothing in it can run);
//! anything else is shown as plain text. Pictures with relative paths are shown
//! from the captured files, so they work offline; relative links open the file
//! on the project's site.

use pulldown_cmark::{html, Options, Parser};
use std::borrow::Cow;

/// Where a readme's relative addresses lead.
pub struct Bases {
    /// The captured version's folder as a library:// address, ending in "/".
    pub files: String,
    /// The same version on the project's site, ending in "/" ("" when unknown).
    pub web: String,
    /// The readme's folder inside the version ("" or "docs/").
    pub dir: String,
}

pub fn to_html(name: &str, bytes: &[u8], bases: &Bases) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdown")) {
        return format!("<pre class=\"doc-text\">{}</pre>", escape(&text));
    }
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_FOOTNOTES | Options::ENABLE_GFM;
    let mut raw = String::new();
    html::push_html(&mut raw, Parser::new_ext(&text, opts));
    clean(&raw, bases)
}

fn clean(raw: &str, bases: &Bases) -> String {
    let files = bases.files.clone();
    let web = bases.web.clone();
    let dir = bases.dir.clone();
    let mut b = ammonia::Builder::default();
    b.add_generic_attributes(["align"])
        .add_tag_attributes("img", ["width", "height", "align"])
        .add_tag_attributes("td", ["align"])
        .add_tag_attributes("th", ["align"])
        .add_url_schemes(["library"])
        .rm_tags(["source"])
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            let relative = !value.contains(':') && !value.starts_with('#') && !value.starts_with("//") && !value.is_empty();
            match (element, attribute) {
                ("img", "src") if relative => join(&files, &dir, value).map(Cow::Owned),
                ("a", "href") if relative => {
                    if web.is_empty() {
                        None
                    } else {
                        join(&web, &dir, value).map(Cow::Owned)
                    }
                }
                _ => Some(Cow::Borrowed(value)),
            }
        });
    b.clean(raw).to_string()
}

/// `base` + `dir` + a relative `path` ("../img/a.png", "./shot.png"), with ".."
/// worked out; None when it climbs out of the version's folder.
fn join(base: &str, dir: &str, path: &str) -> Option<String> {
    let (path, tail) = match path.find(['?', '#']) {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, ""),
    };
    let mut parts: Vec<&str> = if path.starts_with('/') { vec![] } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    // a picture's address is a file in the library: keep its query out
    let tail = if base.starts_with("library:") || base.contains("library.localhost") || base.starts_with('/') { "" } else { tail };
    Some(format!("{base}{}{tail}", parts.join("/")))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bases() -> Bases {
        Bases {
            files: "library://localhost/github.com/o/r/source/2026-10-29_0325_abc1234/".into(),
            web: "https://github.com/o/r/blob/abc1234/".into(),
            dir: "".into(),
        }
    }

    #[test]
    fn markdown_with_html_is_cleaned() {
        let md = b"# Tool\n\n<p align=\"center\"><img src=\"docs/logo.png\" width=\"120\"></p>\n\n<script>alert(1)</script>\n\n[guide](docs/GUIDE.md) [site](https://x.org) [bad](javascript:alert(1)) [top](#tool)\n\n![shot](./shots/a.png?raw=true)";
        let h = to_html("README.md", md, &bases());
        assert!(h.contains("<h1>Tool</h1>"), "{h}");
        assert!(!h.contains("<script"), "{h}");
        assert!(h.contains("src=\"library://localhost/github.com/o/r/source/2026-10-29_0325_abc1234/docs/logo.png\""), "{h}");
        assert!(h.contains("width=\"120\"") && h.contains("align=\"center\""), "{h}");
        assert!(h.contains("href=\"https://github.com/o/r/blob/abc1234/docs/GUIDE.md\""), "{h}");
        assert!(h.contains("href=\"https://x.org\"") && !h.contains("javascript:"), "{h}");
        assert!(h.contains("href=\"#tool\""), "{h}");
        assert!(h.contains("source/2026-10-29_0325_abc1234/shots/a.png\""), "{h}");
    }

    #[test]
    fn plain_text_and_paths() {
        assert_eq!(to_html("README", b"<b>", &bases()), "<pre class=\"doc-text\">&lt;b&gt;</pre>");
        assert_eq!(join("B/", "docs/", "../img/a.png").as_deref(), Some("B/img/a.png"));
        assert_eq!(join("B/", "", "../../a.png"), None);
        assert_eq!(join("https://w/", "", "a.md#x").as_deref(), Some("https://w/a.md#x"));
    }
}
