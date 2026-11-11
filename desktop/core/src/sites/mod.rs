//! Hosting a project's website on this computer (docs/PLAN.md, "Hosting
//! websites"): what a captured version holds as a site ([`detect`]), how a
//! request maps to a file the way GitHub Pages does it ([`router`]), the server
//! itself ([`server`]), saving the files a site loads from other servers so it
//! works offline ([`vendor`], [`rewrite`]), capturing a live website
//! ([`mirror`]), exporting a version as a repository ([`export`]) and building a
//! site on this computer ([`build`]).
//!
//! Everything here reads the library's folders and writes only derived data
//! (`<project>/site/…` and `_library/sites/`): the captured versions themselves
//! are never changed.

pub mod build;
pub mod detect;
pub mod export;
pub mod mime;
pub mod mirror;
pub mod rewrite;
pub mod router;
pub mod scan;
pub mod server;
pub mod vendor;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What kind of site a captured version is, as detected (or chosen in Site settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiteKind {
    /// Not a website (or nothing says so).
    #[default]
    None,
    /// Files served as they are (an Actions deployment of plain files, or `.nojekyll`).
    Static,
    /// Deployed from a branch without Jekyll features: served as they are, with
    /// Jekyll's hiding rules and Markdown pages shown as HTML.
    JekyllPassthrough,
    /// A Jekyll site with layouts or front matter: it has to be built.
    Jekyll,
    /// A site built by a tool (npm, Hugo, MkDocs…) when it's published.
    Build,
    /// A workflow builds the site and pushes it to a branch (gh-pages).
    Branch,
    /// A website captured from the internet.
    Mirror,
}

impl SiteKind {
    pub fn is_site(self) -> bool {
        self != SiteKind::None
    }
    /// Whether the version's own files can be served (no build needed).
    pub fn servable(self) -> bool {
        matches!(self, SiteKind::Static | SiteKind::JekyllPassthrough | SiteKind::Mirror)
    }
    pub fn label(self) -> &'static str {
        match self {
            SiteKind::None => "Not a website",
            SiteKind::Static => "Static website",
            SiteKind::JekyllPassthrough => "Website (published from a branch)",
            SiteKind::Jekyll => "Jekyll website (built when published)",
            SiteKind::Build => "Website built when published",
            SiteKind::Branch => "Website published to a branch",
            SiteKind::Mirror => "Captured website",
        }
    }
}

/// How a site that needs building is built (from its workflow or its files).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildHint {
    /// "npm", "jekyll", "hugo", "mkdocs", "make"…
    pub tool: String,
    pub command: String,
    /// The folder the build writes the site into.
    pub output: String,
}

/// What a captured version holds as a website: detected from its files, kept on
/// the snapshot in repo.json (it can always be worked out again).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteInfo {
    pub kind: SiteKind,
    /// The folder the site is published from, relative to the version ("" for all of it).
    pub root: String,
    /// The address the site lives under ("/name/" for a project site, "/" with a custom domain).
    pub base: String,
    /// The first page ("index.html", or "README.md" for a Markdown-only site).
    pub entry: String,
    pub has_404: bool,
    pub cname: String,
    /// Looks like a single-page app that routes in the browser.
    pub spa: bool,
    pub nojekyll: bool,
    /// Why it was taken for this kind, in words.
    pub evidence: Vec<String>,
    /// Things that may not work ("files kept in Git LFS are missing").
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildHint>,
    /// The branch a workflow publishes the built site to ("gh-pages").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
}

/// How a site that needs building is built on this computer (Site settings).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildSettings {
    /// "" for the detected command.
    pub command: String,
    /// "" for the detected output folder.
    pub output: String,
    /// The user agreed to run this project's own code on this computer.
    pub approved: bool,
}

/// How a website is captured (a mirrored website, or a project's live site).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MirrorSettings {
    /// Only pages under the start address's folder.
    pub same_folder: bool,
    /// How many links away from the start page to follow.
    pub depth: u32,
    pub max_pages: u32,
    /// Other servers whose files are kept with the site (besides the usual CDNs).
    pub hosts: Vec<String>,
    /// Paths not to capture ("/forum/*").
    pub skip: Vec<String>,
}

impl Default for MirrorSettings {
    fn default() -> Self {
        Self { same_folder: true, depth: 6, max_pages: 400, hosts: vec![], skip: vec![] }
    }
}

/// A project's website settings (repo.json, "site"): what the user chose, over
/// what was detected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteSettings {
    /// None follows detection; true or false is the user's choice.
    pub enabled: Option<bool>,
    /// The name in the site's address (`<label>.localhost`), given once and kept.
    pub label: String,
    /// "auto", or a kind: "static", "jekyll-passthrough", "pages-branch", "live", "build".
    pub kind: String,
    /// "" for the detected folder.
    pub root: String,
    /// "" for the detected base path.
    pub base: String,
    /// "auto", "none" or "index" (unknown addresses get index.html, for apps that route in the browser).
    pub spa_fallback: String,
    /// The version served at the main address ("<code|pages|live>/<folder>"), "" for the newest.
    pub pin: String,
    /// Save the files the site loads from other servers after each capture.
    pub offline: bool,
    /// Serve pages with a policy that stops them reaching the internet (to test offline use).
    pub block_internet: bool,
    /// Files missing from a capture are fetched from the live site while you browse it.
    pub recording: bool,
    /// Extra servers whose files are kept, and servers never to fetch from.
    pub extra_hosts: Vec<String>,
    pub skip_hosts: Vec<String>,
    /// Extra response headers (Cross-Origin-Opener-Policy…).
    pub headers: BTreeMap<String, String>,
    /// The live address (for captures of the deployed site).
    pub live_url: String,
    pub build: BuildSettings,
    pub mirror: MirrorSettings,
}

impl Default for SiteSettings {
    fn default() -> Self {
        Self {
            enabled: None,
            label: String::new(),
            kind: "auto".into(),
            root: String::new(),
            base: String::new(),
            spa_fallback: "auto".into(),
            pin: String::new(),
            offline: true,
            block_internet: false,
            recording: true,
            extra_hosts: vec![],
            skip_hosts: vec![],
            headers: BTreeMap::new(),
            live_url: String::new(),
            build: BuildSettings::default(),
            mirror: MirrorSettings::default(),
        }
    }
}

impl SiteSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Which list of a project's versions a site version is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Coll {
    /// The followed branch (source/), or a mirrored website's captures.
    Code,
    /// The branch the site is published to (pages/).
    Pages,
    /// Captures of the project's live site (live/).
    Live,
}

impl Coll {
    pub fn dir(self) -> &'static str {
        match self {
            Coll::Code => "source",
            Coll::Pages => "pages",
            Coll::Live => "live",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Coll::Code => "code",
            Coll::Pages => "pages",
            Coll::Live => "live",
        }
    }
    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "code" | "source" => Some(Coll::Code),
            "pages" => Some(Coll::Pages),
            "live" => Some(Coll::Live),
            _ => None,
        }
    }
    /// The start of a version's own address ("", "p-", "l-").
    pub fn prefix(self) -> &'static str {
        match self {
            Coll::Code => "",
            Coll::Pages => "p-",
            Coll::Live => "l-",
        }
    }
}

/// One DNS label: lower-case letters, digits and "-", at most 63 characters.
pub fn dns_label(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let mut out: String = out.chars().take(63).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "site".into()
    } else {
        out
    }
}

/// The address name a project's site gets: "<name>.<owner>" for GitHub,
/// "<name>.<owner>.<host>" for other servers, "<page>.<domain>.web" for a mirrored website.
pub fn default_label(source: &crate::sources::SourceRef) -> String {
    use crate::sources::Kind;
    let name = dns_label(source.name());
    let owner = dns_label(&source.owner().replace('/', "-"));
    match source.kind {
        Kind::Web => format!("{name}.{owner}.web"),
        _ if source.host == "github.com" => format!("{name}.{owner}"),
        _ => format!("{name}.{owner}.{}", dns_label(&source.host)),
    }
}

/// The short code in a version's own address: the end of its folder name
/// ("2026-11-10_0930_f8cb196" -> "f8cb196").
pub fn version_code(folder: &str) -> String {
    let tail = folder.trim_end_matches('_').rsplit('_').next().unwrap_or(folder);
    dns_label(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{Kind, SourceRef};

    #[test]
    fn labels_are_valid_hostnames() {
        let r = |kind, host: &str, path: &str| default_label(&SourceRef { kind, host: host.into(), path: path.into() });
        assert_eq!(r(Kind::GitHub, "github.com", "Agilancer/vectorclad"), "vectorclad.agilancer");
        assert_eq!(r(Kind::GitLab, "gitlab.com", "Group/Sub/My_Proj"), "my-proj.group-sub.gitlab-com");
        assert_eq!(r(Kind::Web, "web", "agilancer.github.io/vectorclad"), "vectorclad.agilancer-github-io.web");
        assert_eq!(dns_label("--__--"), "site");
        assert_eq!(dns_label(&"x".repeat(80)).len(), 63);
        assert_eq!(version_code("2026-11-10_0930_f8cb196"), "f8cb196");
        assert_eq!(version_code("2026-11-10_0930_f8cb196_"), "f8cb196");
        assert!(SiteSettings::default().is_default());
    }
}
