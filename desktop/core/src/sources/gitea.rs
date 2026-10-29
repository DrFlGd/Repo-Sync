//! Gitea and Forgejo (Codeberg and self-hosted servers), API v1.

use super::{s, topics, Asset, Commit, Details, Release, Source, SourceRef};
use crate::http::{Client, Header};
use crate::util::percent_encode;
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

pub struct Gitea {
    r: SourceRef,
    api: String,
    token: Option<String>,
    client: Arc<Client>,
}

impl Gitea {
    pub fn new(r: SourceRef, api: String, token: Option<String>, client: Arc<Client>) -> Self {
        Self { r, api, token, client }
    }

    fn headers_for(&self, url: &str) -> Vec<Header> {
        let mut h = vec![("Accept", "application/json".to_string())];
        let ours = url.starts_with(&self.api) || url.starts_with(&format!("https://{}/", self.r.host));
        if let (Some(t), true) = (&self.token, ours) {
            h.push(("Authorization", format!("token {t}")));
        }
        h
    }

    fn get(&self, path: &str, what: &str) -> Result<Value> {
        let url = format!("{}/repos/{}{path}", self.api, self.r.path);
        self.client.json(&url, &self.headers_for(&url), "the server", what)
    }
}

impl Source for Gitea {
    fn details(&self) -> Result<Details> {
        let v = self.get("", &self.r.path)?;
        let license = v["licenses"].as_array().and_then(|a| a.first()).map(s).unwrap_or_default();
        Ok(Details {
            name: s(&v["name"]),
            description: s(&v["description"]),
            homepage: s(&v["website"]),
            web_url: s(&v["html_url"]),
            default_branch: s(&v["default_branch"]),
            stars: v["stars_count"].as_u64().unwrap_or(0),
            forks: v["forks_count"].as_u64().unwrap_or(0),
            license,
            language: s(&v["language"]),
            topics: topics(&v["topics"]),
            archived: v["archived"].as_bool().unwrap_or(false),
            pushed_at: s(&v["updated_at"]),
            created_at: s(&v["created_at"]),
        })
    }

    fn commit(&self, branch: &str) -> Result<Commit> {
        let v = self.get(&format!("/branches/{}", percent_encode(branch)), &format!("the {branch} branch of {}", self.r.path))?;
        let c = &v["commit"];
        Ok(Commit {
            sha: c["id"].as_str().context("the server returned no commit id")?.to_string(),
            date: s(&c["timestamp"]),
            message: s(&c["message"]),
            author: s(&c["author"]["name"]),
        })
    }

    fn source_zip(&self, sha: &str) -> (String, Vec<Header>) {
        let url = format!("{}/repos/{}/archive/{sha}.zip", self.api, self.r.path);
        let h = self.headers_for(&url);
        (url, h)
    }

    fn latest_release(&self, prereleases: bool) -> Result<Option<Release>> {
        let list = self.get("/releases?limit=10&draft=false", "the releases")?;
        let Some(arr) = list.as_array() else { return Ok(None) };
        let pick = arr.iter().find(|r| !r["draft"].as_bool().unwrap_or(false) && (prereleases || !r["prerelease"].as_bool().unwrap_or(false)));
        Ok(pick.map(|v| Release {
            tag: s(&v["tag_name"]),
            name: s(&v["name"]),
            published: s(&v["published_at"]),
            prerelease: v["prerelease"].as_bool().unwrap_or(false),
            notes: s(&v["body"]),
            web_url: s(&v["html_url"]),
            assets: v["assets"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|x| Asset { name: s(&x["name"]), size: x["size"].as_u64().unwrap_or(0), url: s(&x["browser_download_url"]), updated: s(&x["created_at"]) })
                        .collect()
                })
                .unwrap_or_default(),
        }))
    }

    fn asset_download(&self, asset: &Asset) -> (String, Vec<Header>) {
        let mut h = self.headers_for(&asset.url);
        h.retain(|(k, _)| *k != "Accept");
        (asset.url.clone(), h)
    }
}
