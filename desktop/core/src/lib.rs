//! Repo Sync's core: the library folder, the sources (GitHub, GitLab, Gitea and
//! Forgejo), capturing versions and releases, readmes, launching, and the app's
//! commands. No window here: desktop/src-tauri puts one around it, and
//! `reposync-cli serve` puts a local web server around it for the tests.

pub mod api;
pub mod archive;
pub mod capture;
pub mod config;
pub mod http;
pub mod launch;
pub mod library;
pub mod readme;
pub mod sources;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
