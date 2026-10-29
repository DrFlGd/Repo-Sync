//! Accounts: a GitHub user or organisation, a GitLab user or group, a Gitea or
//! Forgejo user or organisation. Every project an account owns can be added at
//! once, and an account can be followed so its new projects are added too.

use super::{api_of, kind_of_host, s, token_for, Kind, Tokens};
use crate::http::{Client, Header};
use crate::util::percent_encode;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub kind: Kind,
    pub host: String,
    pub name: String,
}

impl Account {
    /// "github.com/DrFlGd".
    pub fn id(&self) -> String {
        format!("{}/{}", self.host, self.name)
    }
    pub fn web_url(&self) -> String {
        format!("https://{}/{}", self.host, self.name)
    }
}

/// One project an account owns.
#[derive(Clone, Debug, Serialize)]
pub struct Listed {
    /// "owner/name" (GitLab: "group/sub/name").
    pub path: String,
    pub fork: bool,
    pub archived: bool,
}

/// GitHub's own pages, which aren't accounts.
const NOT_ACCOUNTS: &[&str] = &["about", "explore", "features", "issues", "login", "marketplace", "new", "notifications", "pricing", "pulls", "search", "settings", "sponsors", "topics", "trending", "users", "orgs", "groups", "dashboard", "-"];

/// Read a link to an account ("https://github.com/DrFlGd", "github.com/orgs/rust-lang",
/// "https://codeberg.org/forgejo"), on a server the app knows by its name.
pub fn parse_account(input: &str) -> Option<Account> {
    let s = input.trim().split(['?', '#']).next().unwrap_or("");
    if s.is_empty() || s.contains(char::is_whitespace) || s.starts_with("git@") {
        return None;
    }
    let r = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let (host, rest) = r.split_once('/')?;
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let kind = kind_of_host(&host)?;
    let mut segs: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if segs.len() == 2 && matches!(segs[0], "orgs" | "users" | "groups") {
        segs.remove(0);
    }
    let [name] = segs[..] else { return None };
    let name = name.strip_prefix('@').unwrap_or(name);
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !NOT_ACCOUNTS.contains(&name.to_ascii_lowercase().as_str());
    ok.then(|| Account { kind, host, name: name.to_string() })
}

/// Every project the account owns (public ones, and private ones a token in
/// Settings can see on GitLab and Gitea), at most 1000.
pub fn projects(a: &Account, tokens: &Tokens, client: &Client) -> Result<Vec<Listed>> {
    let api = api_of(a.kind, &a.host);
    let token = token_for(tokens, &a.host);
    let headers: Vec<Header> = match (a.kind, &token) {
        (Kind::GitHub, Some(t)) => vec![("Accept", "application/vnd.github+json".into()), ("Authorization", format!("Bearer {t}"))],
        (Kind::GitHub, None) => vec![("Accept", "application/vnd.github+json".into())],
        (Kind::GitLab, Some(t)) => vec![("PRIVATE-TOKEN", t.clone())],
        (Kind::Gitea, Some(t)) => vec![("Authorization", format!("token {t}"))],
        _ => vec![],
    };
    let site = a.kind.label();
    let what = format!("the projects of {}", a.name);
    // a page at a time, until a short page
    let all = |base: &str, per: usize| -> Result<Option<Vec<Value>>> {
        let sep = if base.contains('?') { '&' } else { '?' };
        let mut out = vec![];
        for page in 1..=(1000 / per) {
            let url = format!("{base}{sep}{}={per}&page={page}", if a.kind == Kind::Gitea { "limit" } else { "per_page" });
            let Some(v) = client.json_opt(&url, &headers, site, &what)? else { return Ok(None) };
            let list = v.as_array().cloned().unwrap_or_default();
            let n = list.len();
            out.extend(list);
            if n < per {
                break;
            }
        }
        Ok(Some(out))
    };
    let name = percent_encode(&a.name);
    let list = match a.kind {
        Kind::GitHub => all(&format!("{api}/users/{name}/repos?type=owner&sort=full_name"), 100)?,
        Kind::Gitea => match all(&format!("{api}/users/{name}/repos"), 50)? {
            Some(l) => Some(l),
            None => all(&format!("{api}/orgs/{name}/repos"), 50)?,
        },
        Kind::GitLab => {
            // a user's own projects, or a group's (with its subgroups)
            let users = client.json(&format!("{api}/users?username={name}"), &headers, site, &what)?;
            match users.as_array().and_then(|u| u.first()).and_then(|u| u["id"].as_u64()) {
                Some(id) => all(&format!("{api}/users/{id}/projects?order_by=path&sort=asc"), 100)?,
                None => all(&format!("{api}/groups/{name}/projects?include_subgroups=true&order_by=path&sort=asc"), 100)?,
            }
        }
    };
    let list = list.with_context(|| format!("{site} has no account called {}.", a.name))?;
    let out: Vec<Listed> = list
        .iter()
        .map(|v| Listed {
            path: match a.kind {
                Kind::GitLab => s(&v["path_with_namespace"]),
                _ => s(&v["full_name"]),
            },
            fork: v["fork"].as_bool().unwrap_or(false) || v["forked_from_project"].is_object(),
            archived: v["archived"].as_bool().unwrap_or(false),
        })
        .filter(|l| l.path.contains('/'))
        .collect();
    if out.is_empty() && list.is_empty() {
        // an empty account is fine; a wrong answer isn't
        return Ok(out);
    }
    if out.is_empty() {
        bail!("{site}'s list of {}'s projects couldn't be read.", a.name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_links() {
        let a = parse_account("https://github.com/DrFlGd").unwrap();
        assert_eq!((a.kind, a.host.as_str(), a.name.as_str()), (Kind::GitHub, "github.com", "DrFlGd"));
        assert_eq!(parse_account("github.com/orgs/rust-lang/").unwrap().name, "rust-lang");
        assert_eq!(parse_account("https://github.com/DrFlGd?tab=repositories").unwrap().name, "DrFlGd");
        assert_eq!(parse_account("https://codeberg.org/forgejo").unwrap().kind, Kind::Gitea);
        assert_eq!(parse_account("https://gitlab.com/groups/gitlab-org").unwrap().kind, Kind::GitLab);
        assert!(parse_account("https://github.com/DrFlGd/terrain-press").is_none());
        assert!(parse_account("https://github.com/settings").is_none());
        assert!(parse_account("https://example.com/someone").is_none());
        assert!(parse_account("DrFlGd").is_none());
    }
}
