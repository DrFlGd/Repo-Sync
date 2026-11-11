//! Building a site that's built when published (Vite, Next, Hugo, MkDocs,
//! Jekyll…) on this computer, only after the user agrees to run the project's
//! own code: a scratch copy of the version is built with the tools installed
//! here, and the site it makes is kept beside the version as `build/`; the
//! captured version is never changed (docs/PLAN.md, "Local builds").

use super::{BuildHint, BuildSettings};
use crate::http::Progress;
use crate::util::{now_iso, write_json};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// What a build will run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub tool: String,
    pub command: String,
    pub output: String,
    /// Programs the command needs that aren't installed here.
    pub missing: Vec<String>,
}

/// `build.json` beside a built version.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub started: String,
    pub finished: String,
    pub ok: bool,
    pub tool: String,
    pub command: String,
    pub output: String,
    pub seconds: u64,
    pub exit: Option<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// The programs a tool's builds need.
fn programs(tool: &str, command: &str) -> Vec<&'static str> {
    let mut p: Vec<&'static str> = match tool {
        "npm" => vec!["node", "npm"],
        "pnpm" => vec!["node", "pnpm"],
        "yarn" => vec!["node", "yarn"],
        "hugo" => vec!["hugo"],
        "mkdocs" => vec!["mkdocs"],
        "mdbook" => vec!["mdbook"],
        "jekyll" => vec!["bundle", "ruby"],
        _ => vec![],
    };
    for (word, prog) in [("npx ", "npx"), ("make", "make"), ("python", "python"), ("bundle ", "bundle"), ("cargo ", "cargo")] {
        if command.contains(word) && !p.contains(&prog) {
            p.push(prog);
        }
    }
    p
}

/// Whether `prog` is on the PATH.
pub fn installed(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let exts: Vec<String> = if cfg!(windows) { std::env::var("PATHEXT").unwrap_or(".EXE;.CMD;.BAT".into()).split(';').map(|s| s.to_ascii_lowercase()).collect() } else { vec![String::new()] };
    std::env::split_paths(&path).any(|d| exts.iter().any(|e| d.join(format!("{prog}{e}")).is_file()))
}

/// What building this version would run, from the user's settings over what was
/// detected. None when there's nothing to build with.
pub fn plan(hint: Option<&BuildHint>, settings: &BuildSettings) -> Option<Plan> {
    let custom = !settings.command.trim().is_empty();
    // a command of the user's own: only the programs it names are looked for
    let tool = if custom { String::new() } else { hint.map(|h| h.tool.clone()).unwrap_or_default() };
    let command = if custom { settings.command.trim().to_string() } else { hint.map(|h| h.command.clone()).unwrap_or_default() };
    let output = if !settings.output.trim().is_empty() { settings.output.trim().to_string() } else { hint.map(|h| h.output.clone()).unwrap_or_default() };
    if command.is_empty() || output.is_empty() {
        return None;
    }
    let missing = if custom { vec![] } else { programs(&tool, &command).into_iter().filter(|p| !installed(p)).map(String::from).collect() };
    Some(Plan { tool, command, output, missing })
}

/// The variables a build gets: what tools need to run, nothing else (no tokens).
fn allowed_env(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    matches!(
        n.as_str(),
        "PATH" | "HOME" | "USERPROFILE" | "APPDATA" | "LOCALAPPDATA" | "PROGRAMDATA" | "PROGRAMFILES" | "PROGRAMFILES(X86)" | "SYSTEMROOT" | "SYSTEMDRIVE" | "WINDIR" | "COMSPEC" | "PATHEXT" | "TEMP" | "TMP" | "TMPDIR" | "LANG" | "LC_ALL" | "USER" | "USERNAME" | "SHELL" | "NUMBER_OF_PROCESSORS" | "PROCESSOR_ARCHITECTURE" | "OS" | "HTTP_PROXY" | "HTTPS_PROXY" | "NO_PROXY" | "NODE_EXTRA_CA_CERTS" | "SSL_CERT_FILE" | "GEM_HOME" | "GEM_PATH" | "NVM_DIR" | "VOLTA_HOME" | "PNPM_HOME"
    ) || n.starts_with("LC_")
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n == ".git" || n == "node_modules" {
            continue;
        }
        let t = e.file_type()?;
        if t.is_dir() {
            copy_tree(&e.path(), &to.join(&name))?;
        } else if t.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(0x0800_0000).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-KILL", &format!("-{pid}")]).status();
    }
}

/// Build the version in `src` with `plan`, keeping the made site in
/// `<into>/build/`, the log in `<into>/build.log` and the record in
/// `<into>/build.json`. `into` is the version's folder under `site/`.
pub fn run(src: &Path, into: &Path, plan: &Plan, progress: &Progress, stage: &crate::capture::Stage) -> Result<Record> {
    if !plan.missing.is_empty() {
        bail!("Building this site needs {}, which isn't installed on this computer.", plan.missing.join(" and "));
    }
    let out_rel = crate::util::safe_rel(Path::new(&plan.output)).context("the output folder must be inside the project")?;
    std::fs::create_dir_all(into)?;
    let work = into.join("work");
    let _ = std::fs::remove_dir_all(&work);
    *stage.lock().unwrap() = "Copying the version to build it".into();
    copy_tree(src, &work).context("couldn't copy the version to build it")?;
    let log_path = into.join("build.log");
    let mut log = std::fs::File::create(&log_path)?;
    writeln!(log, "Repo Sync build, {}\n$ {}\n", now_iso(), plan.command)?;
    let started = now_iso();
    let t0 = Instant::now();

    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt;
        // cmd reads its own command line: with /S it drops the outer quotes and runs the rest as typed
        let mut c = Command::new("cmd");
        c.raw_arg(format!("/D /S /C \"{}\"", plan.command));
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let mut c = Command::new("sh");
        c.args(["-c", &plan.command]);
        c
    };
    c.current_dir(&work).env_clear();
    for (k, v) in std::env::vars_os() {
        if allowed_env(&k.to_string_lossy()) {
            c.env(k, v);
        }
    }
    c.env("CI", "1").env("NODE_ENV", "production").env("JEKYLL_ENV", "production");
    c.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log.try_clone()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    *stage.lock().unwrap() = format!("Building: {}", plan.command);
    let mut child = c.spawn().with_context(|| format!("couldn't start {}", plan.command))?;
    let mut rec = Record { started, tool: plan.tool.clone(), command: plan.command.clone(), output: plan.output.clone(), ..Default::default() };
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if progress.stopped() {
            kill_tree(child.id());
            let _ = child.wait();
            rec.why = "Stopped.".into();
            break None;
        }
        if t0.elapsed() > TIMEOUT {
            kill_tree(child.id());
            let _ = child.wait();
            rec.why = "The build took over 30 minutes and was stopped.".into();
            break None;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    rec.seconds = t0.elapsed().as_secs();
    rec.finished = now_iso();
    rec.exit = status.and_then(|s| s.code());
    let made = work.join(&out_rel);
    if status.is_some_and(|s| s.success()) {
        if made.join("index.html").is_file() || made.is_dir() && std::fs::read_dir(&made).map(|mut d| d.next().is_some()).unwrap_or(false) {
            let dest = into.join("build");
            let _ = std::fs::remove_dir_all(&dest);
            std::fs::rename(&made, &dest).or_else(|_| copy_tree(&made, &dest)).context("couldn't keep the built site")?;
            rec.ok = true;
        } else {
            rec.why = format!("The build finished but made nothing in {}.", plan.output);
        }
    } else if rec.why.is_empty() {
        rec.why = format!("The build failed{}. Its log says what went wrong.", rec.exit.map(|c| format!(" (exit code {c})")).unwrap_or_default());
    }
    let _ = writeln!(log, "\n{}", if rec.ok { "Built." } else { rec.why.as_str() });
    drop(log);
    let _ = std::fs::remove_dir_all(&work);
    write_json(&into.join("build.json"), &serde_json::to_value(&rec)?)?;
    Ok(rec)
}

pub fn read_record(into: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(into.join("build.json")).ok()?).ok()
}

/// The end of a build's log, for the Site tab.
pub fn log_tail(into: &Path, max: usize) -> String {
    let t = std::fs::read_to_string(into.join("build.log")).unwrap_or_default();
    let n = t.chars().count();
    if n > max {
        t.chars().skip(n - max).collect()
    } else {
        t
    }
}

/// Where a version's build is kept.
pub fn built_root(into: &Path) -> Option<PathBuf> {
    let b = into.join("build");
    (b.is_dir() && read_record(into).is_some_and(|r| r.ok)).then_some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_runs_in_a_copy_and_keeps_its_output() {
        let d = std::env::temp_dir().join(format!("reposync-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/page.txt"), "hello").unwrap();
        let cmd = if cfg!(windows) { "mkdir out && copy page.txt out\\index.html" } else { "mkdir out && cp page.txt out/index.html && echo built" };
        let plan = Plan { tool: "make".into(), command: cmd.into(), output: "out".into(), missing: vec![] };
        let stage = crate::capture::Stage::default();
        let rec = run(&d.join("src"), &d.join("site"), &plan, &Progress::default(), &stage).unwrap();
        assert!(rec.ok, "{rec:?}");
        assert_eq!(std::fs::read_to_string(d.join("site/build/index.html")).unwrap().trim(), "hello");
        assert!(!d.join("src/out").exists() && !d.join("site/work").exists());
        assert!(built_root(&d.join("site")).is_some());
        let bad = Plan { command: "exit 3".into(), ..plan };
        let rec = run(&d.join("src"), &d.join("site2"), &bad, &Progress::default(), &stage).unwrap();
        assert!(!rec.ok && rec.exit == Some(3));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_override_detection() {
        let h = BuildHint { tool: "hugo".into(), command: "hugo --minify".into(), output: "public".into() };
        let p = plan(Some(&h), &BuildSettings { output: "docs".into(), ..Default::default() }).unwrap();
        assert_eq!((p.command.as_str(), p.output.as_str()), ("hugo --minify", "docs"));
        assert!(plan(None, &BuildSettings::default()).is_none());
    }
}
