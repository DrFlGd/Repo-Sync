//! Web requests: JSON from a source's API, and downloads to a file with progress
//! and Stop.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Progress of the download in hand, read by the Activity page.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    /// 0 when the size isn't known.
    pub total: AtomicU64,
    pub stop: AtomicBool,
}

impl Progress {
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

pub type Shared = Arc<Progress>;

/// The app's web client: one agent, so connections are kept between requests.
pub struct Client {
    agent: ureq::Agent,
    /// For this computer and hosts in NO_PROXY: never through a proxy.
    direct: ureq::Agent,
}

/// A header for one request (name, value).
pub type Header = (&'static str, String);

impl Client {
    pub fn new() -> Self {
        let build = |proxy: bool| {
            ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(20))
                .timeout_read(std::time::Duration::from_secs(120))
                .try_proxy_from_env(proxy)
                .user_agent(&format!("Repo-Sync/{}", crate::VERSION))
                .build()
        };
        Self { agent: build(true), direct: build(false) }
    }

    fn get(&self, url: &str, headers: &[Header]) -> ureq::Request {
        let agent = if no_proxy(url) { &self.direct } else { &self.agent };
        let mut r = agent.get(url);
        for (k, v) in headers {
            r = r.set(k, v);
        }
        r
    }

    /// GET and read JSON. `what` names the thing asked for, for error messages.
    pub fn json(&self, url: &str, headers: &[Header], site: &str, what: &str) -> Result<Value> {
        let r = self.get(url, headers).call().map_err(|e| explain(e, site, what))?;
        let text = r.into_string().with_context(|| format!("couldn't read {site}'s answer for {what}"))?;
        serde_json::from_str(&text).with_context(|| format!("{site}'s answer for {what} wasn't JSON"))
    }

    /// GET and read JSON, or None when the server says it doesn't exist (404).
    pub fn json_opt(&self, url: &str, headers: &[Header], site: &str, what: &str) -> Result<Option<Value>> {
        match self.get(url, headers).call() {
            Ok(r) => {
                let text = r.into_string()?;
                Ok(Some(serde_json::from_str(&text).with_context(|| format!("{site}'s answer for {what} wasn't JSON"))?))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(explain(e, site, what)),
        }
    }

    /// Download to `dest`, counting into `progress` and stopping when asked.
    /// `limit` (bytes, 0 for none) stops a download that grows past it.
    #[allow(clippy::too_many_arguments)]
    pub fn download(&self, url: &str, headers: &[Header], dest: &Path, progress: &Progress, limit: u64, site: &str, what: &str) -> Result<u64> {
        let r = self.get(url, headers).call().map_err(|e| explain(e, site, what))?;
        let total = r.header("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        progress.total.store(total, Ordering::Relaxed);
        progress.done.store(0, Ordering::Relaxed);
        if limit > 0 && total > limit {
            bail!("{what} is {} MB, over the limit in Settings ({} MB).", total >> 20, limit >> 20);
        }
        let mut reader = r.into_reader();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(dest).with_context(|| format!("couldn't create {}", dest.display()))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut n: u64 = 0;
        loop {
            if progress.stopped() {
                drop(f);
                let _ = std::fs::remove_file(dest);
                bail!("Stopped.");
            }
            let k = reader.read(&mut buf).with_context(|| format!("the download of {what} broke off"))?;
            if k == 0 {
                break;
            }
            f.write_all(&buf[..k]).with_context(|| format!("couldn't write {}", dest.display()))?;
            n += k as u64;
            progress.done.store(n, Ordering::Relaxed);
            if limit > 0 && n > limit {
                drop(f);
                let _ = std::fs::remove_file(dest);
                bail!("{what} is over the limit in Settings ({} MB).", limit >> 20);
            }
        }
        f.sync_all().ok();
        if total > 0 && n != total {
            let _ = std::fs::remove_file(dest);
            bail!("the download of {what} stopped short ({n} of {total} bytes)");
        }
        Ok(n)
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `url` is on this computer or a host NO_PROXY names.
fn no_proxy(url: &str) -> bool {
    let host = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = host.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if host.starts_with('[') { host.split(']').next().unwrap_or("").trim_start_matches('[') } else { host.split(':').next().unwrap_or("") };
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || host == "::1" {
        return true;
    }
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|e| if e.trim() == "*" { "*".to_string() } else { e.trim().trim_start_matches('*').trim_start_matches('.').to_ascii_lowercase() }).filter(|e| !e.is_empty()).any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

/// What a source's server said, in words for the user.
fn explain(e: ureq::Error, site: &str, what: &str) -> anyhow::Error {
    match e {
        ureq::Error::Status(404, _) => anyhow!("{site} says {what} doesn't exist (or it's private: add a token for {site} in Settings)."),
        ureq::Error::Status(401, _) => anyhow!("{site} didn't accept the token in Settings."),
        ureq::Error::Status(403 | 429, r) => {
            let reset = r.header("x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok());
            let remaining = r.header("x-ratelimit-remaining").map(|v| v == "0").unwrap_or(false);
            match reset {
                Some(t) if remaining || r.status() == 429 => anyhow!(
                    "{site}'s hourly request limit is used up (until {} UTC). A token in Settings raises it.",
                    &crate::util::iso_from_unix(t)[11..16]
                ),
                _ => anyhow!("{site} refused the request for {what}."),
            }
        }
        ureq::Error::Status(code, _) => anyhow!("{site} answered {code} for {what}."),
        ureq::Error::Transport(t) => anyhow!("Couldn't reach {site}: {t}"),
    }
}
