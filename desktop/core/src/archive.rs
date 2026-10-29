//! Unpacking ZIPs (a captured version of the code, a release's ZIP before it's
//! launched) and .tar.gz files. Only plain files with plain relative paths come
//! out: no "..", no absolute paths, no links.

use crate::http::Progress;
use crate::util::safe_rel;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Unpack `zip` into `dest`. With `strip_top`, a single folder holding
/// everything (GitHub's "owner-name-1a2b3c4/") is left out of the paths.
pub fn unzip(zip: &Path, dest: &Path, strip_top: bool, progress: Option<&Progress>) -> Result<u64> {
    let f = std::fs::File::open(zip).with_context(|| format!("couldn't open {}", zip.display()))?;
    let mut z = zip::ZipArchive::new(f).with_context(|| format!("{} isn't a ZIP that can be read", zip.display()))?;
    let names: Vec<PathBuf> = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().and_then(|e| e.enclosed_name())).collect();
    let top = if strip_top { common_top(&names) } else { None };
    std::fs::create_dir_all(dest)?;
    let mut count = 0;
    for i in 0..z.len() {
        if progress.is_some_and(|p| p.stopped()) {
            bail!("Stopped.");
        }
        let mut e = z.by_index(i)?;
        let Some(name) = e.enclosed_name() else { continue };
        let name = match &top {
            Some(t) => match name.strip_prefix(t) {
                Ok(r) if !r.as_os_str().is_empty() => r.to_path_buf(),
                _ => continue,
            },
            None => name,
        };
        let Some(rel) = safe_rel(&name) else { continue };
        let out = dest.join(&rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        // a link inside the archive is skipped (it could point anywhere)
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out).with_context(|| format!("couldn't write {}", out.display()))?;
        std::io::copy(&mut e, &mut w).with_context(|| format!("couldn't unpack {}", rel.display()))?;
        #[cfg(unix)]
        if let Some(m) = e.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            if m & 0o111 != 0 {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755));
            }
        }
        count += 1;
    }
    Ok(count)
}

/// Unpack a .tar.gz (or .tgz) into `dest`.
pub fn untar_gz(file: &Path, dest: &Path) -> Result<u64> {
    let f = std::fs::File::open(file).with_context(|| format!("couldn't open {}", file.display()))?;
    let mut t = tar::Archive::new(flate2::read::GzDecoder::new(f));
    std::fs::create_dir_all(dest)?;
    let mut count = 0;
    for e in t.entries()? {
        let mut e = e?;
        let kind = e.header().entry_type();
        let Some(rel) = e.path().ok().and_then(|p| safe_rel(&p)) else { continue };
        let out = dest.join(&rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else if kind.is_file() {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut buf = vec![];
            e.read_to_end(&mut buf)?;
            std::fs::write(&out, buf)?;
            #[cfg(unix)]
            if e.header().mode().is_ok_and(|m| m & 0o111 != 0) {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755));
            }
            count += 1;
        }
    }
    Ok(count)
}

/// The one folder every entry is inside, if there is one.
fn common_top(names: &[PathBuf]) -> Option<PathBuf> {
    let mut top: Option<&std::ffi::OsStr> = None;
    for n in names {
        let mut c = n.components();
        let first = c.next()?.as_os_str();
        // a file at the top: nothing to strip
        if c.next().is_none() && !names.iter().any(|m| m != n && m.starts_with(n)) {
            return None;
        }
        match top {
            None => top = Some(first),
            Some(t) if t != first => return None,
            _ => {}
        }
    }
    top.map(PathBuf::from)
}

#[cfg(test)]
pub(crate) fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unzip_strips_the_top_folder_and_refuses_escapes() {
        let dir = std::env::temp_dir().join(format!("reposync-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let z = dir.join("a.zip");
        make_zip(&z, &[("o-r-abc/README.md", b"# Hi"), ("o-r-abc/src/main.rs", b"fn main(){}"), ("../evil.txt", b"x")]);
        let n = unzip(&z, &dir.join("out"), true, None).unwrap();
        assert_eq!(n, 2);
        assert!(dir.join("out/README.md").is_file() && dir.join("out/src/main.rs").is_file());
        assert!(!dir.join("evil.txt").exists());
        // no common folder: nothing stripped
        let z2 = dir.join("b.zip");
        make_zip(&z2, &[("tool.exe", b"MZ"), ("data/x.bin", b"1")]);
        unzip(&z2, &dir.join("out2"), true, None).unwrap();
        assert!(dir.join("out2/tool.exe").is_file() && dir.join("out2/data/x.bin").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
