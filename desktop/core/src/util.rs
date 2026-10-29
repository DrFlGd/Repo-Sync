//! Small helpers shared by the rest of the core: times, JSON files, folder names
//! and sizes.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Seconds since 1970 (UTC).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "2026-10-29T03:25:00Z" for a time in seconds since 1970.
pub fn iso_from_unix(t: i64) -> String {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

pub fn now_iso() -> String {
    iso_from_unix(now_unix())
}

/// Seconds since 1970 from an ISO 8601 time ("2026-10-29T03:25:00Z",
/// "2026-10-29T03:25:00.123+02:00"). None if it can't be read.
pub fn unix_from_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = if s.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    let mut t = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se;
    // a zone after the seconds (and any fraction): Z, +hh:mm or -hh:mm
    let rest = s.get(19..).unwrap_or("");
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let z = &rest[1..];
        let zh = z.get(0..2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let zm = z.get(z.len().saturating_sub(2)..).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let off = zh * 3600 + if z.len() > 2 { zm * 60 } else { 0 };
        t -= if sign == '+' { off } else { -off };
    }
    Some(t)
}

// Howard Hinnant's date algorithms.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-10-29_0325" (UTC), the start of a captured version's folder name.
pub fn stamp_from_unix(t: i64) -> String {
    let iso = iso_from_unix(t);
    format!("{}_{}{}", &iso[0..10], &iso[11..13], &iso[14..16])
}

/// A JSON object from a file, or {} if it's missing or unreadable.
pub fn read_json_object(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()))
}

/// Write via a temporary file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, data).with_context(|| format!("couldn't write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(())
}

/// Pretty JSON with sorted keys and a final newline, so a library kept in git or
/// a sync tool changes only where something changed.
pub fn write_json(path: &Path, v: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(&sorted(v))?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&m[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        v => v.clone(),
    }
}

/// A name that is safe as a folder or file name on Windows, macOS and Linux.
pub fn safe_name(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    out = out.trim().trim_end_matches(['.', ' ']).to_string();
    let upper = out.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    if matches!(stem, "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out = "_".into();
    }
    out.chars().take(120).collect()
}

/// A relative path with plain parts only (no "..", no root, no drive), or None.
pub fn safe_rel(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Bytes and files under a folder (links aren't followed).
pub fn dir_size(dir: &Path) -> (u64, u64) {
    let (mut bytes, mut files) = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// "%2F" and friends back to characters (paths in library:// addresses).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode one part of an address.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        let t = 1_793_244_300; // 2026-10-29T03:25:00Z
        assert_eq!(iso_from_unix(t), "2026-10-29T03:25:00Z");
        assert_eq!(unix_from_iso("2026-10-29T03:25:00Z"), Some(t));
        assert_eq!(unix_from_iso("2026-10-29T05:25:00.123+02:00"), Some(t));
        assert_eq!(unix_from_iso("2026-10-28T22:25:00-05:00"), Some(t));
        assert_eq!(stamp_from_unix(t), "2026-10-29_0325");
        assert_eq!(unix_from_iso("soon"), None);
    }

    #[test]
    fn names_are_safe_everywhere() {
        assert_eq!(safe_name("v1.0/beta:2"), "v1.0_beta_2");
        assert_eq!(safe_name("con"), "_con");
        assert_eq!(safe_name("COM1.txt"), "_COM1.txt");
        assert_eq!(safe_name("  dots... "), "dots");
        assert_eq!(safe_rel(Path::new("../x")), None);
        assert_eq!(safe_rel(Path::new("a/./b")), Some(PathBuf::from("a/b")));
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    }
}
