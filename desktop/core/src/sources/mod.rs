//! Where repositories come from. Each kind of server (GitHub, GitLab, Gitea and
//! Forgejo, which Codeberg runs) answers the same five questions through
//! [`Source`]: the project's details, the newest commit on a branch, where to get
//! that commit as a ZIP, the latest release, and where to get a release file.
//! Adding another kind of server means one more file here (docs/PLAN.md,
//! "Sources").

mod gitea;
mod github;
mod gitlab;

use crate::http::{Client, Header};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    GitHub,
    GitLab,
    Gitea,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::GitHub => "GitHub",
            Kind::GitLab => "GitLab",
            Kind::Gitea => "Gitea / Forgejo",
        }
    }
}

/// One project on one server: `host` and its `path` ("owner/name", or
/// "group/subgroup/name" on GitLab).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub kind: Kind,
    pub host: String,
    pub path: String,
}

impl SourceRef {
    /// The project's id in the library, which is also its folder: "github.com/owner/name".
    pub fn id(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
    pub fn owner(&self) -> &str {
        self.path.rsplit_once('/').map(|(o, _)| o).unwrap_or("")
    }
    pub fn web_url(&self) -> String {
        format!("https://{}/{}", self.host, self.path)
    }
}

/// A link as typed or pasted, read into a project and the kind of server when the
/// host says it (None: an unknown host, found out with [`probe`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub kind: Option<Kind>,
    pub host: String,
    pub path: String,
}

impl Parsed {
    pub fn with_kind(&self, kind: Kind) -> SourceRef {
        SourceRef { kind, host: self.host.clone(), path: self.path.clone() }
    }
}

/// Read "https://github.com/owner/name", ".../tree/main/docs", "github.com/owner/name",
/// "git@github.com:owner/name.git", "owner/name" (GitHub), GitLab links with groups
/// ("https://gitlab.com/group/sub/name/-/releases") and Codeberg or other Gitea links.
pub fn parse_url(input: &str) -> Result<Parsed> {
    let s = input.trim();
    if s.is_empty() {
        bail!("Paste a link to a repository, such as https://github.com/owner/name.");
    }
    let s = s.split(['?', '#']).next().unwrap_or("");
    if s.contains(char::is_whitespace) {
        bail!("That doesn't look like a link to a repository.");
    }
    let (host, rest) = if let Some(r) = s.strip_prefix("git@") {
        match r.split_once(':') {
            Some((h, p)) => (h.to_string(), p.to_string()),
            None => bail!("That doesn't look like a repository link."),
        }
    } else {
        let r = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
        let parts: Vec<&str> = r.split('/').collect();
        if parts.len() == 2 && !parts[0].contains('.') && !r.contains(':') {
            // "owner/name": GitHub
            ("github.com".to_string(), r.to_string())
        } else {
            match r.split_once('/') {
                Some((h, p)) => (h.to_string(), p.to_string()),
                None => bail!("That link has no project in it: add the owner and the name, as in https://{r}/owner/name."),
            }
        }
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || "-.:".contains(c)) {
        bail!("That doesn't look like a repository link.");
    }
    let kind = kind_of_host(&host);
    let mut segs: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    match kind {
        Some(Kind::GitLab) => {
            // GitLab puts its own pages after "/-/"; before that, groups then the project
            if let Some(i) = segs.iter().position(|p| *p == "-") {
                segs.truncate(i);
            }
        }
        _ => segs.truncate(2),
    }
    if let Some(last) = segs.last_mut() {
        *last = last.strip_suffix(".git").unwrap_or(last);
    }
    if segs.len() < 2 {
        bail!("The link needs both the owner and the project's name: https://{host}/owner/name");
    }
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !segs.iter().all(|p| ok(p)) {
        bail!("{} isn't a valid project name.", segs.join("/"));
    }
    Ok(Parsed { kind, host, path: segs.join("/") })
}

fn kind_of_host(host: &str) -> Option<Kind> {
    let first = host.split('.').next().unwrap_or("");
    match host {
        "github.com" => Some(Kind::GitHub),
        "gitlab.com" | "salsa.debian.org" | "gitlab.gnome.org" | "invent.kde.org" | "gitlab.freedesktop.org" => Some(Kind::GitLab),
        "codeberg.org" | "gitea.com" | "git.disroot.org" | "next.forgejo.org" => Some(Kind::Gitea),
        _ if first == "gitlab" => Some(Kind::GitLab),
        _ if first == "gitea" || first == "forgejo" => Some(Kind::Gitea),
        _ => None,
    }
}

/// A project's details, as the library keeps them in repo.json ("details").
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Details {
    pub name: String,
    pub description: String,
    pub homepage: String,
    pub web_url: String,
    pub default_branch: String,
    pub stars: u64,
    pub forks: u64,
    pub license: String,
    pub language: String,
    pub topics: Vec<String>,
    pub archived: bool,
    /// When the server last saw a change (a push), ISO 8601.
    pub pushed_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub date: String,
    pub message: String,
    pub author: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    /// 0 when the server doesn't say.
    pub size: u64,
    pub url: String,
    pub updated: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub published: String,
    pub prerelease: bool,
    pub notes: String,
    pub web_url: String,
    pub assets: Vec<Asset>,
}

/// The questions every kind of server answers.
pub trait Source: Send + Sync {
    fn details(&self) -> Result<Details>;
    /// The newest commit on `branch`.
    fn commit(&self, branch: &str) -> Result<Commit>;
    /// Where to download `sha` as a ZIP (one top folder holding the files).
    fn source_zip(&self, sha: &str) -> (String, Vec<Header>);
    /// The latest release (or pre-release, with `prereleases`), None when there are none.
    fn latest_release(&self, prereleases: bool) -> Result<Option<Release>>;
    /// The project's releases (pre-releases too, never drafts), newest first, at most `limit`.
    fn releases(&self, limit: usize) -> Result<Vec<Release>>;
    /// The names of the project's branches.
    fn branches(&self) -> Result<Vec<String>>;
    /// Where to download one of a release's files.
    fn asset_download(&self, asset: &Asset) -> (String, Vec<Header>);
}

/// Tokens by host ("github.com" -> "ghp_..."), from Settings.
pub type Tokens = BTreeMap<String, String>;

/// The API address for a host: `REPOSYNC_API` ("host=url;host=url") replaces the
/// real one (the tests point it at tests/forge_mock.py).
fn api_base(host: &str, default: String) -> String {
    if let Ok(over) = std::env::var("REPOSYNC_API") {
        for pair in over.split(';') {
            if let Some((h, u)) = pair.split_once('=') {
                if h.trim().eq_ignore_ascii_case(host) {
                    return u.trim().trim_end_matches('/').to_string();
                }
            }
        }
    }
    default
}

pub fn open(r: &SourceRef, tokens: &Tokens, client: Arc<Client>) -> Box<dyn Source> {
    let token = tokens.get(&r.host).filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string());
    match r.kind {
        Kind::GitHub => {
            let default = if r.host == "github.com" { "https://api.github.com".to_string() } else { format!("https://{}/api/v3", r.host) };
            Box::new(github::GitHub::new(r.clone(), api_base(&r.host, default), token, client))
        }
        Kind::GitLab => Box::new(gitlab::GitLab::new(r.clone(), api_base(&r.host, format!("https://{}/api/v4", r.host)), token, client)),
        Kind::Gitea => Box::new(gitea::Gitea::new(r.clone(), api_base(&r.host, format!("https://{}/api/v1", r.host)), token, client)),
    }
}

/// For a host the app doesn't know: ask it as Gitea, then as GitLab.
pub fn probe(p: &Parsed, tokens: &Tokens, client: Arc<Client>) -> Result<SourceRef> {
    if let Some(k) = p.kind {
        return Ok(p.with_kind(k));
    }
    for kind in [Kind::Gitea, Kind::GitLab] {
        let r = p.with_kind(kind);
        if open(&r, tokens, client.clone()).details().is_ok() {
            return Ok(r);
        }
    }
    bail!("{} doesn't answer like GitHub, GitLab, Gitea or Forgejo, or {} isn't there.", p.host, p.path)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn topics(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(u: &str) -> (Option<Kind>, String, String) {
        let r = parse_url(u).unwrap();
        (r.kind, r.host, r.path)
    }

    #[test]
    fn links_of_every_shape() {
        let gh = |path: &str| (Some(Kind::GitHub), "github.com".to_string(), path.to_string());
        assert_eq!(p("https://github.com/DrFlGd/Game-Repo"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("https://github.com/DrFlGd/Game-Repo/"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("https://www.github.com/DrFlGd/Game-Repo/tree/main/docs?x=1#y"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("https://github.com/DrFlGd/Game-Repo/releases/tag/v0.5.54"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("github.com/DrFlGd/Game-Repo.git"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("git@github.com:DrFlGd/Game-Repo.git"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("DrFlGd/Game-Repo"), gh("DrFlGd/Game-Repo"));
        assert_eq!(p("https://gitlab.com/group/sub/proj/-/releases"), (Some(Kind::GitLab), "gitlab.com".into(), "group/sub/proj".into()));
        assert_eq!(p("https://codeberg.org/forgejo/forgejo/releases"), (Some(Kind::Gitea), "codeberg.org".into(), "forgejo/forgejo".into()));
        assert_eq!(p("https://git.example.net/me/thing"), (None, "git.example.net".into(), "me/thing".into()));
        assert!(parse_url("https://github.com/DrFlGd").is_err());
        assert!(parse_url("").is_err());
        assert!(parse_url("hello there").is_err());
        assert!(parse_url("https://github.com/a/..").is_err());
    }

    #[test]
    fn ids_and_names() {
        let r = SourceRef { kind: Kind::GitLab, host: "gitlab.com".into(), path: "group/sub/proj".into() };
        assert_eq!(r.id(), "gitlab.com/group/sub/proj");
        assert_eq!(r.name(), "proj");
        assert_eq!(r.owner(), "group/sub");
        assert_eq!(r.web_url(), "https://gitlab.com/group/sub/proj");
    }
}
