//! Which release files are for which system, and launching a release: finding the
//! programs in a release's folder (unpacking a ZIP or .tar.gz beside it first),
//! choosing the best one for this computer, and starting it.

use crate::archive;
use crate::util::safe_rel;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Mac,
    Android,
}

pub fn this_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// The systems a release file's name says it's for (empty: any, such as checksums or data).
pub fn systems_of(name: &str) -> Vec<Os> {
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &[&str]| words.iter().any(|x| w.contains(x));
    let ends = |e: &[&str]| e.iter().any(|x| n.ends_with(x));
    let mut out = vec![];
    if has(&["win", "win32", "win64", "windows", "msvc", "mingw"]) || ends(&[".exe", ".msi", ".msix", ".appx", ".bat", ".cmd"]) {
        out.push(Os::Windows);
    }
    if has(&["linux", "ubuntu", "debian", "fedora", "musl", "gnu", "appimage", "flatpak", "steamdeck"]) || ends(&[".appimage", ".deb", ".rpm", ".flatpak", ".snap"]) {
        out.push(Os::Linux);
    }
    if has(&["mac", "macos", "osx", "darwin", "apple"]) || ends(&[".dmg", ".pkg"]) {
        out.push(Os::Mac);
    }
    if has(&["android"]) || ends(&[".apk", ".aab"]) {
        out.push(Os::Android);
    }
    out
}

/// Whether a release file is for this computer: it names no system, or names this
/// one, and doesn't name another processor than this one.
pub fn for_this_computer(name: &str) -> bool {
    let s = systems_of(name);
    if !(s.is_empty() || s.contains(&this_os())) {
        return false;
    }
    let n = name.to_ascii_lowercase();
    let words: Vec<&str> = n.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').flat_map(|w| w.split('_')).filter(|w| !w.is_empty()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w)) || list.iter().any(|x| x.contains('_') && n.contains(x));
    if has(&["freebsd", "openbsd", "netbsd", "solaris", "illumos", "aix"]) {
        return false;
    }
    let x64 = has(&["x64", "x86_64", "amd64", "win64", "64bit"]) || n.contains("x86-64");
    let arm64 = has(&["arm64", "aarch64"]);
    let other = has(&["armv6", "armv7", "armhf", "armel", "arm32", "arm", "386", "i386", "i686", "ppc64le", "ppc64", "s390x", "riscv64", "mips", "mips64", "mipsle", "loong64", "32bit"]);
    let mine = if cfg!(target_arch = "aarch64") { arm64 } else { x64 };
    let foreign = other || if cfg!(target_arch = "aarch64") { x64 } else { arm64 };
    // a name that names this processor is kept even when it names others too ("x64-arm64" bundles)
    mine || !foreign
}

/// Something in a release's folder that Launch can start.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    /// Relative to the release's folder, with "/".
    pub file: String,
    /// "program", "installer", "script" or "archive" (unpacked, then its program started).
    pub kind: &'static str,
    pub score: i32,
}

fn ext(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".tar.gz") {
        return "tar.gz".into();
    }
    n.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default()
}

fn kind_of(path: &Path, os: Os) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_ascii_lowercase();
    let installer = ["setup", "install", "installer"].iter().any(|w| lower.contains(w));
    let e = ext(&name);
    let k = match (os, e.as_str()) {
        (Os::Windows, "exe") => if installer { "installer" } else { "program" },
        (Os::Windows, "msi" | "msix" | "appx") => "installer",
        (Os::Windows, "bat" | "cmd" | "ps1") => "script",
        (Os::Linux, "appimage") => "program",
        (Os::Linux, "sh") => "script",
        (Os::Linux, "deb" | "rpm" | "flatpak") => "installer",
        (Os::Mac, "dmg" | "pkg") => "installer",
        (Os::Mac, "sh" | "command") => "script",
        (_, "jar") => "program",
        (_, "zip" | "tar.gz" | "tgz") => "archive",
        #[cfg(unix)]
        (Os::Linux | Os::Mac, "") => {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(path).ok()?;
            if m.is_file() && m.permissions().mode() & 0o111 != 0 { "program" } else { return None }
        }
        _ => return None,
    };
    Some(k)
}

/// Files Launch could start in a release's folder, best first. `project` (the
/// project's name) counts in a file's favour.
pub fn candidates(folder: &Path, project: &str) -> Vec<Candidate> {
    let os = this_os();
    let proj = project.to_ascii_lowercase().replace(['-', '_', ' '], "");
    let mut out = vec![];
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 4 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let Some(kind) = kind_of(&p, os) else { continue };
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !for_this_computer(&name) {
                continue;
            }
            let rel = p.strip_prefix(folder).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            // an archive that has been unpacked already is shown by its programs
            if kind == "archive" && unpacked_dir(&p).is_dir() {
                continue;
            }
            let mut score = match kind {
                "program" => 50,
                "installer" => 30,
                "script" => 20,
                _ => 10,
            };
            if name.replace(['-', '_', ' '], "").contains(&proj) && !proj.is_empty() {
                score += 8;
            }
            if ["unins", "uninstall", "crash", "helper", "update", "redist", "vc_", "debug"].iter().any(|w| name.contains(w)) {
                score -= 40;
            }
            if !systems_of(&name).is_empty() {
                score += 3; // made for this system, not just any
            }
            // a portable build is started as it is, which is what Launch is for: before an installer
            if kind == "archive" && (name.contains("portable") || !systems_of(&name).is_empty()) {
                score += 25;
            }
            score -= depth as i32; // nearer the top is more likely the program itself
            out.push(Candidate { file: rel, kind, score });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.file.cmp(&b.file)));
    out
}

/// Where an archive in a release is unpacked: a folder beside it, named after it.
pub fn unpacked_dir(archive_path: &Path) -> PathBuf {
    let name = archive_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let stem = [".tar.gz", ".tgz", ".zip"].iter().find(|e| lower.ends_with(*e)).map(|e| &name[..name.len() - e.len()]).unwrap_or(&name);
    archive_path.with_file_name(stem)
}

/// What was started.
#[derive(Debug, Serialize)]
pub struct Launched {
    pub file: String,
    pub pid: u32,
}

/// Start `file` (relative to the release's `folder`), or the best candidate when
/// it's empty. An archive is unpacked beside itself first and its best program started.
pub fn launch(folder: &Path, file: &str, project: &str) -> Result<Launched> {
    let pick = if file.is_empty() {
        candidates(folder, project).into_iter().next().map(|c| c.file).context("This release has nothing to launch on this computer.")?
    } else {
        file.to_string()
    };
    let rel = safe_rel(Path::new(&pick)).context("not a file in the release")?;
    let path = folder.join(&rel);
    if !path.is_file() {
        bail!("{pick} isn't in the release's folder any more.");
    }
    if kind_of(&path, this_os()) == Some("archive") {
        let dest = unpacked_dir(&path);
        if !dest.is_dir() {
            let tmp = dest.with_extension("unpacking");
            let _ = std::fs::remove_dir_all(&tmp);
            if ext(&pick) == "zip" {
                archive::unzip(&path, &tmp, false, None)?;
            } else {
                archive::untar_gz(&path, &tmp)?;
            }
            std::fs::rename(&tmp, &dest)?;
        }
        let inner = candidates(&dest, project).into_iter().find(|c| c.kind != "archive").context("The archive has nothing to launch on this computer.")?;
        let prefix = dest.strip_prefix(folder).unwrap_or(&dest).to_string_lossy().replace('\\', "/");
        return launch(folder, &format!("{prefix}/{}", inner.file), project);
    }
    let pid = start(&path)?;
    Ok(Launched { file: pick, pid })
}

fn start(path: &Path) -> Result<u32> {
    use std::process::{Command, Stdio};
    let dir = path.parent().unwrap_or(Path::new("."));
    let e = ext(&path.to_string_lossy());
    let mut cmd = match e.as_str() {
        "msi" => {
            let mut c = Command::new("msiexec");
            c.arg("/i").arg(path);
            c
        }
        "bat" | "cmd" => {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(path);
            c
        }
        "ps1" => {
            let mut c = Command::new("powershell");
            c.args(["-ExecutionPolicy", "Bypass", "-File"]).arg(path);
            c
        }
        "jar" => {
            let mut c = Command::new("java");
            c.arg("-jar").arg(path);
            c
        }
        "sh" | "command" => {
            let mut c = Command::new("sh");
            c.arg(path);
            c
        }
        "dmg" | "pkg" | "deb" | "rpm" | "flatpak" | "msix" | "appx" => {
            // handed to the system's installer
            let mut c = if cfg!(windows) {
                let mut c = Command::new("cmd");
                c.args(["/C", "start", ""]);
                c
            } else if cfg!(target_os = "macos") {
                Command::new("open")
            } else {
                Command::new("xdg-open")
            };
            c.arg(path);
            c
        }
        _ => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
            }
            Command::new(path)
        }
    };
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if matches!(e.as_str(), "bat" | "cmd" | "ps1" | "msi") {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    let child = cmd.spawn().with_context(|| format!("couldn't start {}", path.display()))?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_files_by_system() {
        assert_eq!(systems_of("Tool-1.2-windows-x64.zip"), vec![Os::Windows]);
        assert_eq!(systems_of("tool_1.2_amd64.AppImage"), vec![Os::Linux]);
        assert_eq!(systems_of("Tool-1.2-macOS-universal.dmg"), vec![Os::Mac]);
        assert_eq!(systems_of("tool.apk"), vec![Os::Android]);
        assert!(systems_of("SHA256SUMS.txt").is_empty());
        assert!(for_this_computer("SHA256SUMS.txt"));
        let mine = match this_os() {
            Os::Windows => "tool-win64.zip",
            Os::Mac => "tool-macos.dmg",
            _ => "tool-linux-x86_64.tar.gz",
        };
        assert!(for_this_computer(mine));
        assert!(!for_this_computer("tool.apk"));
        if cfg!(target_arch = "x86_64") {
            assert!(!for_this_computer("tool-linux-aarch64.tar.gz"));
            assert!(!for_this_computer("tool-windows-arm64.zip"));
            assert!(!for_this_computer("glab_1.126.0_linux_386.tar.gz"));
            assert!(!for_this_computer("glab_1.126.0_linux_s390x.deb"));
            assert!(for_this_computer("glab_1.126.0_Windows_x86_64_installer.exe") == cfg!(windows));
            assert!(for_this_computer("glab_1.126.0_linux_amd64.deb") == cfg!(target_os = "linux"));
        }
        assert!(!for_this_computer("glab_1.126.0_freebsd_amd64.tar.gz"));
        assert!(for_this_computer("glab_1.126.0_source.tar.gz"));
        assert!(for_this_computer("checksums.txt"));
    }

    #[test]
    fn candidates_prefer_the_program_and_unpack_archives() {
        let dir = std::env::temp_dir().join(format!("reposync-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (prog, other) = if cfg!(windows) { ("demo.cmd", "demo-linux.sh") } else { ("demo.sh", "demo-windows.cmd") };
        let script: &[u8] = if cfg!(windows) { b"@echo launched> launched.txt\r\n" } else { b"echo launched > launched.txt\n" };
        archive::make_zip(&dir.join("demo-portable.zip"), &[(prog, script)]);
        std::fs::write(dir.join(other), b"x").unwrap();
        std::fs::write(dir.join("SHA256SUMS"), b"x").unwrap();
        let c = candidates(&dir, "demo");
        assert_eq!(c.len(), 1, "{c:?}");
        // a portable ZIP comes before an installer
        let installer = if cfg!(windows) { "demo-1.0-setup.exe" } else { "demo_1.0_amd64.deb" };
        std::fs::write(dir.join(installer), b"x").unwrap();
        let c = candidates(&dir, "demo");
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0].kind, "archive");
        std::fs::remove_file(dir.join(installer)).unwrap();
        let c = candidates(&dir, "demo");
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].kind, "archive");
        let l = launch(&dir, "", "demo").unwrap();
        assert_eq!(l.file, format!("demo-portable/{prog}"));
        // the script writes beside itself
        let marker = dir.join("demo-portable/launched.txt");
        for _ in 0..50 {
            if marker.is_file() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(marker.is_file());
        // unpacked: listed by what's inside, not as the archive
        let c = candidates(&dir, "demo");
        assert_eq!(c[0].file, format!("demo-portable/{prog}"));
        assert!(launch(&dir, "../x", "demo").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
