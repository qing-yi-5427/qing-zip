//! Explain failures of split archives whose volumes are missing.
//!
//! The engines only report "Unexpected end of archive" or "Cannot open the file
//! as archive" for a set like `photos.zip.001` .. `.004` with `.002` absent.
//! Name the missing volume instead, from the sibling file names alone.

use std::path::Path;

/// Volume naming schemes: `name.001`, `name.part1.rar`, `name.z01` (+ `name.zip`).
fn parse(file: &str) -> Option<(String, String, usize, usize)> {
    let lower = file.to_ascii_lowercase();
    if let Some((stem, digits)) = lower.rsplit_once('.') {
        if digits.len() >= 3 && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Some((file[..stem.len() + 1].to_owned(), String::new(), digits.parse().ok()?, digits.len()));
        }
        if digits.len() == 3 && digits.starts_with('z') && digits[1..].bytes().all(|b| b.is_ascii_digit()) {
            return Some((file[..stem.len() + 1].to_owned() + "z", String::new(), digits[1..].parse().ok()?, 2));
        }
    }
    let rar = lower.strip_suffix(".rar")?;
    let at = rar.rfind(".part")?;
    let digits = &rar[at + 5..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((file[..at + 5].to_owned(), file[rar.len()..].to_owned(), digits.parse().ok()?, digits.len()))
}

fn name(prefix: &str, suffix: &str, number: usize, width: usize) -> String {
    format!("{prefix}{number:0width$}{suffix}")
}

/// A missing or truncated volume for the set that `path` belongs to, if any.
pub fn problem(path: &str) -> Option<String> {
    let path = Path::new(path);
    let file = path.file_name()?.to_str()?;
    let (prefix, suffix, _, width) = parse(file)?;
    let parent = path.parent()?;
    let mut present = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let Some(other) = entry.file_name().to_str().map(str::to_owned) else { continue };
        if let Some((p, s, number, _)) = parse(&other) {
            if p.eq_ignore_ascii_case(&prefix) && s.eq_ignore_ascii_case(&suffix) {
                present.insert(number);
            }
        }
    }
    let last = *present.last()?;
    if let Some(gap) = (1..last).find(|n| !present.contains(n)) {
        return Some(format!("missing archive volume: {}", name(&prefix, &suffix, gap, width)));
    }
    // WinZip-style sets end with name.zip after name.z01 .. name.zNN.
    if prefix.to_ascii_lowercase().ends_with(".z") {
        let final_name = format!("{}zip", &prefix[..prefix.len() - 1]);
        if !parent.join(&final_name).exists() {
            return Some(format!("missing archive volume: {final_name}"));
        }
        return None;
    }
    Some(format!("archive volumes may be incomplete after: {}", name(&prefix, &suffix, last, width)))
}

/// Replace an engine failure on a split archive with the volume problem, when
/// the failure is what a missing or short volume produces.
pub fn explain(path: &str, error: String) -> String {
    // What a short or absent volume produces; listing failures also echo the
    // first volume's records ("Volume Index").
    let truncated = ["Unexpected end of archive", "Cannot open the file as", "Can't open as archive", "Headers Error"]
        .iter().any(|marker| error.contains(marker))
        || (error.starts_with("archive engine failed") && error.contains("Volume Index"));
    let damaged = truncated || ["CRC Failed", "Data Error"].iter().any(|marker| error.contains(marker));
    match problem(path) {
        // A gap in the numbering explains any damage report.
        Some(problem) if damaged && problem.starts_with("missing") => problem,
        // Without a gap only a truncation report points at a missing last volume.
        Some(problem) if truncated => problem,
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_missing_middle_and_possibly_missing_last_volumes() {
        let dir = tempfile::tempdir().unwrap();
        let touch = |name: &str| std::fs::write(dir.path().join(name), b"x").unwrap();
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        for name in ["v.zip.001", "v.zip.003", "v.zip.004"] { touch(name); }
        assert_eq!(problem(&path("v.zip.001")).unwrap(), "missing archive volume: v.zip.002");
        for name in ["s.7z.001", "s.7z.002"] { touch(name); }
        assert_eq!(problem(&path("s.7z.001")).unwrap(), "archive volumes may be incomplete after: s.7z.002");
        for name in ["r.part1.rar", "r.part3.rar"] { touch(name); }
        assert_eq!(problem(&path("r.part1.rar")).unwrap(), "missing archive volume: r.part2.rar");
        for name in ["w.z01", "w.z02"] { touch(name); }
        assert_eq!(problem(&path("w.z01")).unwrap(), "missing archive volume: w.zip");
        touch("plain.zip");
        assert!(problem(&path("plain.zip")).is_none());
        let engine = "archive engine failed (2): ERRORS:\nUnexpected end of archive".to_string();
        assert_eq!(explain(&path("v.zip.001"), engine.clone()), "missing archive volume: v.zip.002");
        assert_eq!(explain(&path("plain.zip"), engine.clone()), engine);
        // Plain corruption of a complete-looking set is not blamed on volumes.
        assert_eq!(explain(&path("s.7z.001"), "archive CRC verification failed: CRC Failed".into()), "archive CRC verification failed: CRC Failed");
        assert_eq!(explain(&path("v.zip.001"), "ERROR: CRC Failed : a".into()), "missing archive volume: v.zip.002");
        assert_eq!(explain(&path("v.zip.001"), "incorrect password or damaged encrypted archive".into()),
            "incorrect password or damaged encrypted archive");
    }
}
