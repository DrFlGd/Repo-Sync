//! GitHub (github.com, or GitHub Enterprise at <host>/api/v3).

use super::{s, topics, Asset, Commit, Details, Release, Source, SourceRef};
use crate::http::{Client, Header};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

pub struct GitHub {
    r: SourceRef,
    api: String,
    token: Option<String>,
    client: Arc<Client>,
}

impl GitHub {
    pub fn new(r: SourceRef, api: String, token: Option<String>, client: Arc<Client>) -> Self {
        Self { r, api, token, client }
    }

    fn headers(&self, accept: &str) -> Vec<Header> {
        let mut h = vec![("Accept", accept.to_string()), ("X-GitHub-Api-Version", "2022-11-28".to_string())];
        if let Some(t) = &self.token {
            h.push(("Authorization", format!("Bearer {t}")));
        }
        h
    }

    fn get(&self, path: &str, what: &str) -> Result<Value> {
        self.client.json(&format!("{}/repos/{}{path}", self.api, self.r.path), &self.headers("application/vnd.github+json"), "GitHub", what)
    }

    fn release(v: &Value) -> Release {
        Release {
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
                        .map(|x| Asset { name: s(&x["name"]), size: x["size"].as_u64().unwrap_or(0), url: s(&x["url"]), updated: s(&x["updated_at"]) })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

impl Source for GitHub {
    fn details(&self) -> Result<Details> {
        let v = self.get("", &self.r.path)?;
        Ok(Details {
            name: s(&v["name"]),
            description: s(&v["description"]),
            homepage: s(&v["homepage"]),
            web_url: s(&v["html_url"]),
            default_branch: s(&v["default_branch"]),
            stars: v["stargazers_count"].as_u64().unwrap_or(0),
            forks: v["forks_count"].as_u64().unwrap_or(0),
            license: v["license"]["spdx_id"].as_str().filter(|l| *l != "NOASSERTION").map(String::from).unwrap_or_else(|| s(&v["license"]["name"])),
            language: s(&v["language"]),
            topics: topics(&v["topics"]),
            archived: v["archived"].as_bool().unwrap_or(false),
            pushed_at: s(&v["pushed_at"]),
            created_at: s(&v["created_at"]),
        })
    }

    fn commit(&self, branch: &str) -> Result<Commit> {
        let v = self.get(&format!("/commits/{branch}"), &format!("the {branch} branch of {}", self.r.path))?;
        Ok(Commit {
            sha: v["sha"].as_str().context("GitHub returned no commit id")?.to_string(),
            date: s(&v["commit"]["committer"]["date"]),
            message: s(&v["commit"]["message"]),
            author: s(&v["commit"]["author"]["name"]),
        })
    }

    fn source_zip(&self, sha: &str) -> (String, Vec<Header>) {
        (format!("{}/repos/{}/zipball/{sha}", self.api, self.r.path), self.headers("application/vnd.github+json"))
    }

    fn latest_release(&self, prereleases: bool) -> Result<Option<Release>> {
        let url = format!("{}/repos/{}/releases", self.api, self.r.path);
        let h = self.headers("application/vnd.github+json");
        if !prereleases {
            // GitHub's "latest" is the newest release that isn't a draft or a pre-release
            return Ok(self.client.json_opt(&format!("{url}/latest"), &h, "GitHub", "the latest release")?.map(|v| Self::release(&v)));
        }
        let list = self.client.json(&format!("{url}?per_page=10"), &h, "GitHub", "the releases")?;
        Ok(list.as_array().and_then(|a| a.iter().find(|r| !r["draft"].as_bool().unwrap_or(false))).map(Self::release))
    }

    fn asset_download(&self, asset: &Asset) -> (String, Vec<Header>) {
        // the API's address for the file, so it works for private repositories too
        (asset.url.clone(), self.headers("application/octet-stream"))
    }
}
