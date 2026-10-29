//! GitLab (gitlab.com and self-hosted servers), API v4.

use super::{s, topics, Asset, Commit, Details, Release, Source, SourceRef};
use crate::http::{Client, Header};
use crate::util::percent_encode;
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

pub struct GitLab {
    r: SourceRef,
    api: String,
    token: Option<String>,
    client: Arc<Client>,
}

impl GitLab {
    pub fn new(r: SourceRef, api: String, token: Option<String>, client: Arc<Client>) -> Self {
        Self { r, api, token, client }
    }

    fn project(&self) -> String {
        format!("{}/projects/{}", self.api, percent_encode(&self.r.path))
    }

    /// The token goes only to the server it's for, never to a release link elsewhere.
    fn headers_for(&self, url: &str) -> Vec<Header> {
        let mut h = vec![("Accept", "application/json".to_string())];
        let ours = url.starts_with(&self.api) || url.starts_with(&format!("https://{}/", self.r.host));
        if let (Some(t), true) = (&self.token, ours) {
            h.push(("PRIVATE-TOKEN", t.clone()));
        }
        h
    }

    fn get(&self, path: &str, what: &str) -> Result<Value> {
        let url = format!("{}{path}", self.project());
        self.client.json(&url, &self.headers_for(&url), "GitLab", what)
    }

    /// A list the API gives a page at a time, up to `limit` entries.
    fn pages(&self, path: &str, limit: usize, what: &str) -> Result<Vec<Value>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut out = vec![];
        for page in 1.. {
            let v = self.get(&format!("{path}{sep}per_page=100&page={page}"), what)?;
            let a = v.as_array().cloned().unwrap_or_default();
            let n = a.len();
            out.extend(a);
            if n < 100 || out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}

impl Source for GitLab {
    fn details(&self) -> Result<Details> {
        let v = self.get("?license=true", &self.r.path)?;
        // the language GitLab counts most of
        let language = self
            .get("/languages", "the project's languages")
            .ok()
            .and_then(|l| {
                l.as_object().and_then(|m| m.iter().max_by(|a, b| a.1.as_f64().unwrap_or(0.0).total_cmp(&b.1.as_f64().unwrap_or(0.0))).map(|(k, _)| k.clone()))
            })
            .unwrap_or_default();
        let mut t = topics(&v["topics"]);
        if t.is_empty() {
            t = topics(&v["tag_list"]);
        }
        Ok(Details {
            name: s(&v["name"]),
            description: s(&v["description"]),
            homepage: String::new(),
            web_url: s(&v["web_url"]),
            default_branch: s(&v["default_branch"]),
            stars: v["star_count"].as_u64().unwrap_or(0),
            forks: v["forks_count"].as_u64().unwrap_or(0),
            license: v["license"]["nickname"].as_str().filter(|l| !l.is_empty()).map(String::from).unwrap_or_else(|| s(&v["license"]["name"])),
            language,
            topics: t,
            archived: v["archived"].as_bool().unwrap_or(false),
            pushed_at: s(&v["last_activity_at"]),
            created_at: s(&v["created_at"]),
        })
    }

    fn commit(&self, branch: &str) -> Result<Commit> {
        let v = self.get(&format!("/repository/commits/{}", percent_encode(branch)), &format!("the {branch} branch of {}", self.r.path))?;
        Ok(Commit {
            sha: v["id"].as_str().context("GitLab returned no commit id")?.to_string(),
            date: s(&v["committed_date"]),
            message: s(&v["message"]),
            author: s(&v["author_name"]),
        })
    }

    fn source_zip(&self, sha: &str) -> (String, Vec<Header>) {
        let url = format!("{}/repository/archive.zip?sha={sha}", self.project());
        let h = self.headers_for(&url);
        (url, h)
    }

    fn latest_release(&self, prereleases: bool) -> Result<Option<Release>> {
        // GitLab has no pre-releases; an "upcoming" release (dated in the future) is the nearest thing
        Ok(self.releases(50)?.into_iter().find(|r| prereleases || !r.prerelease))
    }

    fn releases(&self, limit: usize) -> Result<Vec<Release>> {
        let list = self.pages("/releases?order_by=released_at&sort=desc", limit, "the releases")?;
        Ok(list
            .iter()
            .take(limit)
            .map(|v| Release {
                tag: s(&v["tag_name"]),
                name: s(&v["name"]),
                published: v["released_at"].as_str().map(String::from).unwrap_or_else(|| s(&v["created_at"])),
                prerelease: v["upcoming_release"].as_bool().unwrap_or(false),
                notes: s(&v["description"]),
                web_url: s(&v["_links"]["self"]),
                assets: v["assets"]["links"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|x| Asset {
                                name: s(&x["name"]),
                                size: 0,
                                url: x["direct_asset_url"].as_str().filter(|u| !u.is_empty()).map(String::from).unwrap_or_else(|| s(&x["url"])),
                                updated: String::new(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect())
    }

    fn branches(&self) -> Result<Vec<String>> {
        Ok(self.pages("/repository/branches", 300, "the branches")?.iter().map(|b| s(&b["name"])).filter(|b| !b.is_empty()).collect())
    }

    fn asset_download(&self, asset: &Asset) -> (String, Vec<Header>) {
        let mut h = self.headers_for(&asset.url);
        h.retain(|(k, _)| *k != "Accept");
        (asset.url.clone(), h)
    }
}
