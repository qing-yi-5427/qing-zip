//! Pure, conservative volume-name grouping; a name is never archive-format proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    Ordinary,
    First {
        family: String,
    },
    Later {
        family: String,
        expected: String,
        first: Option<String>,
    },
}
fn numbered(name: &str) -> Option<(&str, &str, u32)> {
    let (base, number) = name.rsplit_once('.')?;
    if number.len() < 3 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, number, number.parse().ok()?))
}
fn numeric(name: &str) -> Option<(&str, &str, u32)> {
    let (base, digits, number) = numbered(name)?;
    [
        ".7z", ".zip", ".rar", ".tar", ".gz", ".gzip", ".bz2", ".xz", ".tgz", ".tbz2", ".txz",
        ".zst",
    ]
    .iter()
    .any(|suffix| base.ends_with(suffix))
    .then_some((base, digits, number))
}
fn part(name: &str) -> Option<(&str, &str, u32)> {
    let i = name.rfind(".part")?;
    let number = name[i + 5..].strip_suffix(".rar")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((&name[..i], number, number.parse().ok()?))
}
/// Probe possible first members early, without assuming their contents are archives.
pub fn first_probe(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    numbered(&lower).is_some_and(|(_, _, n)| n == 1) || part(&lower).is_some_and(|(_, _, n)| n == 1)
}
/// Discovery suggestions only: these names never authorize cleanup or collapse.
pub fn content_family(first: &str, siblings: &[String]) -> Vec<String> {
    let lower = first.to_ascii_lowercase();
    let generic = numbered(&lower)
        .filter(|(_, _, n)| *n == 1)
        .map(|(base, _, _)| base);
    let first_family = match classify(first, &[]) {
        Member::First { family } => Some(family),
        _ => None,
    };
    siblings.iter().filter(|name|{
        if name.eq_ignore_ascii_case(first){return true;}
        if let Some(base)=generic{if numbered(&name.to_ascii_lowercase()).is_some_and(|(other,_,_)|other==base){return true;}}
            matches!(classify(name,&[]),Member::Later{ref family,ref expected,..}if first_family.as_ref()==Some(family)||expected.eq_ignore_ascii_case(first))
    }).cloned().collect()
}
/// Conservative overwrite protection for names a root decoder can open.
pub fn root_family(first: &str, siblings: &[String]) -> Vec<String> {
    let mut protected = content_family(first, siblings);
    let mut known = protected.iter().cloned().collect::<std::collections::HashSet<_>>();
    let lower = first.to_lowercase();
    let Some((stem, ext)) = lower.rsplit_once('.') else { return protected };
    for sibling in siblings {
        let name = sibling.to_lowercase();
        let Some((base, suffix)) = name.rsplit_once('.') else { continue };
        let same_stem = stem == base;
        let zip_part = matches!(ext, "zip" | "exe")
            && (matches!(suffix, "zip" | "exe")
                || (suffix.len() >= 3 && suffix.starts_with('z') && suffix[1..].bytes().all(|b| b.is_ascii_digit())));
        let old_rar_part = matches!(ext, "rar" | "exe")
            && (matches!(suffix, "rar" | "exe")
                || (suffix.len() == 3 && matches!(suffix.as_bytes()[0], b'r'..=b'z')
                    && suffix.as_bytes()[1..].iter().all(u8::is_ascii_digit)));
        let numbered_part = numbered(&lower).zip(numbered(&name))
            .is_some_and(|((a, _, _), (b, _, _))| a == b);
        let new_rar_part = matches!(ext, "rar" | "exe") && suffix == "rar"
            && rar_number(stem).zip(rar_number(base)).is_some_and(|(a, b)| a == b);
        if ((same_stem && (zip_part || old_rar_part)) || numbered_part || new_rar_part)
            && known.insert(sibling.clone())
        {
            protected.push(sibling.clone());
        }
    }
    protected
}

fn rar_number(stem: &str) -> Option<(&str, &str)> {
    let end = stem.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = stem[..end].rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |index| index + stem[index..].chars().next().unwrap().len_utf8());
    Some((&stem[..start], &stem[end..]))
}

pub fn contains_only_confirmed<T>(files: &[T], confirmed: impl Fn(&T) -> bool) -> bool {
    !files.is_empty() && files.iter().all(confirmed)
}
pub fn classify(name: &str, siblings: &[String]) -> Member {
    let lower = name.to_ascii_lowercase();
    if let Some((base, digits, number)) = part(&lower) {
        let family = format!("rar-part:{base}");
        if number == 1 {
            return Member::First { family };
        }
        let first = siblings
            .iter()
            .find(|candidate| {
                part(&candidate.to_ascii_lowercase()).is_some_and(|(b, _, n)| b == base && n == 1)
            })
            .cloned();
        return Member::Later {
            family,
            expected: format!("{base}.part{:0width$}.rar", 1, width = digits.len()),
            first,
        };
    }
    if let Some((base, digits, number)) = numeric(&lower) {
        let family = format!("numbered:{base}");
        if number == 1 {
            return Member::First { family };
        }
        let first = siblings
            .iter()
            .find(|candidate| {
                numeric(&candidate.to_ascii_lowercase())
                    .is_some_and(|(b, _, n)| b == base && n == 1)
            })
            .cloned();
        return Member::Later {
            family,
            expected: format!("{base}.{:0width$}", 1, width = digits.len()),
            first,
        };
    }
    if let Some((base, tail)) = lower.rsplit_once('.') {
        if tail.len() >= 3
            && matches!(tail.as_bytes()[0], b'z' | b'r')
            && tail.as_bytes()[1..].iter().all(u8::is_ascii_digit)
        {
            let extension = if tail.starts_with('z') { "zip" } else { "rar" };
            let expected = format!("{base}.{extension}");
            let first = siblings
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(&expected))
                .cloned();
            return Member::Later {
                family: format!("{extension}-parts:{base}"),
                expected,
                first,
            };
        }
    }
    Member::Ordinary
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protection_includes_extended_zip_sfx_and_legacy_rar_members() {
        let zip = ["files.zip", "files.z100", "files.exe", "other.z100"].map(str::to_owned);
        let rar = ["files.rar", "files.r00", "files.s00", "other.s00"].map(str::to_owned);
        let modern = ["files001xx.rar", "files002xx.rar", "files002yy.rar"].map(str::to_owned);
        assert_eq!(root_family("files.zip", &zip), zip[..3]);
        assert_eq!(root_family("files.rar", &rar), rar[..3]);
        assert_eq!(root_family("files001xx.rar", &modern), modern[..2]);
    }

    #[test]
    fn absent_first_is_explicit_for_recognized_volume_names() {
        for (name, expected) in [
            ("data.7z.002", "data.7z.001"),
            ("data.z01", "data.zip"),
            ("data.part02.rar", "data.part01.rar"),
        ] {
            assert!(
                matches!(classify(name,&[]),Member::Later{first:None,expected:ref e,..}if e==expected)
            );
        }
    }
    #[test]
    fn complete_family_has_one_first_and_secondary_members_point_to_it() {
        for (first, later) in [
            ("DATA.7z.001", "data.7z.002"),
            ("Data.ZIP", "data.z01"),
            ("DATA.part01.rar", "data.part02.rar"),
        ] {
            let siblings = vec![first.into(), later.into()];
            assert!(
                matches!(classify(later,&siblings),Member::Later{first:Some(ref actual),..}if actual==first)
            );
        }
    }
    #[test]
    fn multiple_orphans_have_one_family_identity() {
        for (a, b) in [
            ("data.7z.002", "data.7z.003"),
            ("data.z01", "data.z02"),
            ("data.part02.rar", "data.part03.rar"),
        ] {
            let family = |name| match classify(name, &[]) {
                Member::Later { family, .. } => family,
                _ => panic!("expected volume"),
            };
            assert_eq!(family(a), family(b));
        }
    }
    #[test]
    fn ordinary_numbered_files_are_not_assumed_to_be_archives() {
        let siblings = vec!["photo.001".into(), "photo.002".into()];
        for name in [
            "photo.001",
            "photo.002",
            "notes.2024",
            "report.part02.txt",
            "file.zebra",
        ] {
            assert_eq!(classify(name, &siblings), Member::Ordinary);
        }
    }
    #[test]
    fn hidden_extension_members_need_first_content_evidence() {
        let mut names = vec![
            "bundle.002".to_owned(),
            "photo.002".to_owned(),
            "bundle.001".to_owned(),
        ];
        assert_eq!(classify("bundle.002", &names), Member::Ordinary);
        names.sort_by_key(|name| !first_probe(name));
        assert_eq!(names[0], "bundle.001");
        let family = content_family("bundle.001", &names);
        assert!(family.contains(&"bundle.002".into()));
        assert!(!family.contains(&"photo.002".into()));
    }
    #[test]
    fn guessed_numeric_companions_never_authorize_cleanup_or_collapse() {
        let files = vec![
            "bundle.001".into(),
            "bundle.002".into(),
            "bundle.009".into(),
        ];
        let proposed = content_family("bundle.001", &files);
        assert_eq!(proposed.len(), 3);
        let confirmed = vec!["bundle.001".to_owned()];
        assert!(!contains_only_confirmed(&files, |name| confirmed.contains(name)));
        assert!(contains_only_confirmed(&confirmed, |name| confirmed.contains(name)));
        assert!(!confirmed.contains(&"bundle.002".into()));
        assert!(!confirmed.contains(&"bundle.009".into()));
    }
}
