//! Shared, read-only archive projection for both file-workspace destinations.
use super::*;
use serde::{Deserialize, Serialize};

// Do not derive Debug: this session-only value contains a source password.
#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub folder: String,
    pub selected: Vec<String>,
    pub password: Option<String>,
    pub codepage: Option<u32>,
}

pub struct Export {
    roots: Vec<String>,
    source_paths: Vec<PathBuf>,
    _directory: ShellDragDirectory,
}
impl Export {
    pub fn roots(&self) -> &[String] {
        &self.roots
    }
    pub(super) fn into_parts(self) -> (ShellDragDirectory, Vec<String>) {
        (self._directory, self.roots)
    }
    pub fn source_paths(&self) -> &[PathBuf] {
        &self.source_paths
    }
}

// Conservative overwrite protection only. These names never authorize a
// decoder to read adjacent files; input-family access stays with with_source.
fn family(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    if ext.len() >= 3 && ext.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("numbered:{stem}"));
    }
    if ext == "zip"
        || (ext.starts_with('z') && ext.len() >= 3 && ext[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        return Some(format!("zip:{stem}"));
    }
    if ext == "rar" {
        if let Some((base, number)) = stem.rsplit_once(".part") {
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
                return Some(format!("rar:{base}"));
            }
        }
        return Some(format!("rar:{stem}"));
    }
    if ext.starts_with('r') && ext.len() >= 3 && ext[1..].bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("rar:{stem}"));
    }
    None
}
fn protected_sources(path: &Path) -> Result<Vec<PathBuf>, String> {
    // Resolve the actual parent and filename once: stripping a verbatim prefix
    // alone does not expand 8.3 aliases. This is only an overwrite deny set.
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let parent = canonical.parent().ok_or("missing source parent")?;
    let mut paths = vec![path.to_owned(), canonical.clone()];
    if let Some(group) = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(family)
    {
        for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name().to_str().and_then(family).as_ref() == Some(&group) {
                paths.push(parent.join(entry.file_name()));
            }
        }
    }
    Ok(paths)
}

fn normalized(name: &str, allow_root: bool) -> Result<String, String> {
    let name = name.replace('\\', "/").trim_end_matches('/').to_owned();
    if name.is_empty() && allow_root {
        return Ok(name);
    }
    validate_name(&name)?;
    Ok(name)
}

fn selections(source: &Source, entries: &[Entry]) -> Result<Vec<String>, String> {
    // Includes unselected members, aliases, stream/link metadata and implicit
    // directory collisions. No decoder sees a rewritten member pathname.
    expected_tree(entries)?;
    let folder = normalized(&source.folder, true)?;
    let prefix = if folder.is_empty() {
        folder
    } else {
        format!("{folder}/")
    };
    if source.selected.is_empty() {
        return Err("请先选择源项目".into());
    }
    let mut available = HashSet::new();
    for entry in entries {
        let mut name = entry
            .name
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_owned();
        available.insert(name.clone());
        while let Some((parent, _)) = name.rsplit_once('/') {
            name = parent.to_owned();
            available.insert(name.clone());
        }
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for selected in &source.selected {
        let name = normalized(selected, false)?;
        let leaf = name
            .strip_prefix(&prefix)
            .ok_or("选中项目不在当前源目录中")?;
        if leaf.is_empty() || leaf.contains('/') {
            return Err("只能复制当前源目录直接显示的项目".into());
        }
        if !available.contains(&name) {
            return Err(format!("源项目不存在：{name}"));
        }
        if seen.insert(name.to_lowercase()) {
            result.push(name);
        }
    }
    Ok(result)
}

/// The entire source is verified once. Source locks end here, before consumers
/// acquire any destination lock; this also permits same-archive directory copy.
pub fn prepare(source: &Source, ctl: &JobControl) -> Result<Export, String> {
    let _policy = crate::machine_policy::scope();
    ctl.check()?;
    let path = absolute(&source.path)?;
    let options = EngineOptions {
        password: source.password.clone(),
        codepage: source.codepage,
        overwrite: "error".into(),
        control: Some(ctl.clone()),
        zone_identifier: Some(crate::motw::archive_context(&path)?),
        ..Default::default()
    };
    let directory = ShellDragDirectory::create_named("qing-zip-archive-export-")?;
    let data = directory.path().join("data");
    let (roots, source_paths) = with_zip_source(
        path.to_str().ok_or("invalid source path")?,
        &options,
        |resolved| {
            let resolved_path = absolute(resolved)?;
            let _pin = crate::engine_stream::pin(&resolved_path)?;
            let _parts = crate::engine_stream::pin_numbered_7z_parts(&resolved_path)?;
            let entries = crate::operations::list_archive(resolved, &options)?;
            let selected = selections(source, &entries)?;
            let mut extraction = options.clone();
            extraction.selected = selected.clone();
            // Native selected extraction already verifies all decoded bytes; adapter
            // routes materialize all members only for this export entry point.
            crate::operations::extract_archive_for_export(
                resolved,
                data.to_str().ok_or("invalid export path")?,
                &extraction,
            )?;
            let mut roots = Vec::new();
            for member in selected {
                ctl.check()?;
                let item = data.join(validate_name(&member)?);
                no_links(&item)?;
                let metadata = fs::symlink_metadata(&item).map_err(|e| e.to_string())?;
                if !metadata.is_file() && !metadata.is_dir() {
                    return Err("源项目未完整解压".into());
                }
                roots.push(item.to_str().ok_or("invalid export name")?.to_owned());
            }
            Ok((roots, protected_sources(&path)?))
        },
    )
    .map_err(|error: String| format!("源压缩包：{error}"))?;
    ctl.check()?;
    Ok(Export {
        roots,
        source_paths,
        _directory: directory,
    })
}

pub fn copy(
    source: &Source,
    target: &str,
    folder: &str,
    opts: &EngineOptions,
) -> Result<(), String> {
    if opts.update_mode != UpdateMode::Add {
        return Err("压缩包复制仅支持保留其他成员的添加模式".into());
    }
    let ctl = control(opts);
    let exported = prepare(source, &ctl)?;
    #[cfg(test)]
    AFTER_PREPARE.with(|hook| {
        if let Some(run) = hook.borrow_mut().take() {
            run(exported.roots());
        }
    });
    ctl.check()?;
    // Only the target options are used here. A missing target password never
    // inherits the source password, even when both packages are encrypted.
    archive_import(target, exported, folder, opts).map_err(|error| format!("目标压缩包：{error}"))
}

#[cfg(test)]
thread_local! {
    static AFTER_PREPARE: std::cell::RefCell<Option<Box<dyn FnOnce(&[String])>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(
        source_format: &str,
        target_format: &str,
        encrypted: bool,
    ) -> (tempfile::TempDir, Source, PathBuf, EngineOptions) {
        let temp = tempfile::tempdir().unwrap();
        let docs = temp.path().join("docs");
        fs::create_dir_all(docs.join("sub/empty")).unwrap();
        fs::write(docs.join("a.txt"), b"source").unwrap();
        fs::write(docs.join("sub/child.txt"), b"child").unwrap();
        let src = temp.path().join(format!("source.{source_format}"));
        let src_opts = EngineOptions {
            password: encrypted.then(|| "source-only-secret".into()),
            header_encryption: if encrypted && source_format == "7z" {
                Some(true)
            } else {
                None
            },
            ..Default::default()
        };
        create(
            src.to_str().unwrap(),
            &[docs.display().to_string()],
            source_format,
            &src_opts,
        )
        .unwrap();
        let backup = temp.path().join("backup");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("a.txt"), b"old").unwrap();
        let outside = temp.path().join("unrelated.txt");
        fs::write(&outside, b"keep").unwrap();
        let dst = temp.path().join(format!("target.{target_format}"));
        let opts = EngineOptions {
            password: encrypted.then(|| "target-only-secret".into()),
            header_encryption: if encrypted && target_format == "7z" {
                Some(true)
            } else {
                None
            },
            ..Default::default()
        };
        create(
            dst.to_str().unwrap(),
            &[backup.display().to_string(), outside.display().to_string()],
            target_format,
            &opts,
        )
        .unwrap();
        (
            temp,
            Source {
                path: src.display().to_string(),
                folder: "docs/".into(),
                selected: vec!["docs/a.txt".into(), "docs/sub/".into()],
                password: src_opts.password,
                codepage: None,
            },
            dst,
            opts,
        )
    }
    #[test]
    fn archive_copy_cross_formats_two_passwords_four_conflicts_and_folder_projection() {
        for (sf, tf, encrypted) in [("zip", "7z", false), ("7z", "zip", true)] {
            for policy in ["error", "skip", "rename", "overwrite"] {
                let (temp, source, target, mut opts) = fixture(sf, tf, encrypted);
                opts.overwrite = policy.into();
                let src_before = fs::read(&source.path).unwrap();
                let target_before = fs::read(&target).unwrap();
                let result = crate::operations::execute_request(crate::operations::Request {
                    operation: "archive-copy".into(),
                    path: target.display().to_string(),
                    dest: "backup/".into(),
                    source_archive: Some(source.clone()),
                    options: opts.clone(),
                    ..Default::default()
                });
                assert_eq!(fs::read(&source.path).unwrap(), src_before);
                if policy == "error" {
                    assert!(result.is_err());
                    assert_eq!(fs::read(&target).unwrap(), target_before);
                    continue;
                }
                result.unwrap();
                let out = temp.path().join("out");
                extract(target.to_str().unwrap(), out.to_str().unwrap(), &opts).unwrap();
                assert_eq!(fs::read(out.join("unrelated.txt")).unwrap(), b"keep");
                assert_eq!(
                    fs::read(out.join("backup/a.txt")).unwrap(),
                    if policy == "overwrite" {
                        b"source".as_slice()
                    } else {
                        b"old".as_slice()
                    }
                );
                assert_eq!(
                    fs::read(out.join("backup/sub/child.txt")).unwrap(),
                    b"child"
                );
                assert!(out.join("backup/sub/empty").is_dir());
                assert!(!out.join("docs").exists());
                assert!(!out.join("a.txt").exists());
                if policy == "rename" {
                    assert_eq!(fs::read(out.join("backup/a (2).txt")).unwrap(), b"source");
                }
            }
        }
    }
    #[test]
    fn archive_copy_same_package_different_directory_and_same_directory_rename() {
        let (temp, source, _, _) = fixture("zip", "zip", false);
        let opts = EngineOptions {
            overwrite: "rename".into(),
            ..Default::default()
        };
        copy(&source, &source.path, "copies/nested", &opts).unwrap();
        copy(&source, &source.path, "docs", &opts).unwrap();
        let out = temp.path().join("out");
        extract(&source.path, out.to_str().unwrap(), &opts).unwrap();
        for name in ["docs/a.txt", "docs/a (2).txt", "copies/nested/a.txt"] {
            assert_eq!(fs::read(out.join(name)).unwrap(), b"source");
        }
        assert!(out.join("copies/nested/sub/empty").is_dir());
    }
    #[test]
    fn archive_copy_wrong_end_passwords_do_not_fall_back_to_other_end_credentials() {
        let (_temp, mut source, target, mut opts) = fixture("zip", "zip", true);
        source.selected = vec!["docs/a.txt".into()];
        opts.overwrite = "overwrite".into();
        let src_before = fs::read(&source.path).unwrap();
        let before = fs::read(&target).unwrap();
        source.password = Some("wrong-source".into());
        let err = copy(&source, target.to_str().unwrap(), "backup", &opts).unwrap_err();
        assert!(err.starts_with("源压缩包"));
        assert!(!err.contains("wrong-source"));
        source.password = Some("source-only-secret".into());
        for password in [None, Some("wrong-target".into())] {
            opts.password = password;
            let err = copy(&source, target.to_str().unwrap(), "backup", &opts).unwrap_err();
            assert!(err.starts_with("目标压缩包"));
            assert!(!err.contains("wrong-target"));
            assert_eq!(fs::read(&target).unwrap(), before);
        }
        assert_eq!(fs::read(&source.path).unwrap(), src_before);
    }
    #[test]
    fn archive_copy_unselected_corruption_and_invalid_projection_never_update_target() {
        let (_temp, mut source, target, opts) = fixture("zip", "zip", false);
        let before = fs::read(&target).unwrap();
        for selection in [
            vec!["elsewhere/a.txt".into()],
            vec!["docs/sub/child.txt".into()],
            vec!["../escape".into()],
        ] {
            source.selected = selection;
            assert!(copy(&source, target.to_str().unwrap(), "backup", &opts).is_err());
            assert_eq!(fs::read(&target).unwrap(), before);
        }
        let mut writer = zip::ZipWriter::new(File::create(&source.path).unwrap());
        for name in ["docs/a.txt", "unused.txt"] {
            writer
                .start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(b"payload").unwrap();
        }
        writer.finish().unwrap();
        let offset = {
            let mut z = zip::ZipArchive::new(File::open(&source.path).unwrap()).unwrap();
            let offset = z.by_name("unused.txt").unwrap().data_start().unwrap();
            offset
        };
        let mut bytes = fs::read(&source.path).unwrap();
        bytes[offset as usize] ^= 1;
        fs::write(&source.path, &bytes).unwrap();
        source.selected = vec!["docs/a.txt".into()];
        assert!(copy(&source, target.to_str().unwrap(), "backup", &opts).is_err());
        assert_eq!(fs::read(&target).unwrap(), before);
        assert_eq!(fs::read(&source.path).unwrap(), bytes);
    }
    #[test]
    fn archive_copy_cancellation_at_both_transaction_boundaries_keeps_packages() {
        for after_engine in [false, true] {
            let (_temp, source, target, mut opts) = fixture("zip", "zip", false);
            opts.overwrite = "overwrite".into();
            let before = fs::read(&target).unwrap();
            let src_before = fs::read(&source.path).unwrap();
            let ctl = JobControl::new();
            let cancel = ctl.clone();
            opts.control = Some(ctl);
            if after_engine {
                update_modes_tests::AFTER_ENGINE
                    .with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
            } else {
                AFTER_PREPARE
                    .with(|hook| *hook.borrow_mut() = Some(Box::new(move |_| cancel.cancel())));
            }
            assert!(copy(&source, target.to_str().unwrap(), "backup", &opts).is_err());
            assert_eq!(fs::read(&target).unwrap(), before);
            assert_eq!(fs::read(&source.path).unwrap(), src_before);
        }
    }
    #[test]
    fn archive_export_family_names_are_only_a_conservative_overwrite_guard() {
        assert_eq!(family("bundle.zip"), family("bundle.z01"));
        assert_eq!(family("bundle.part01.rar"), family("bundle.part02.rar"));
        assert_eq!(family("bundle.rar"), family("bundle.r00"));
        assert_eq!(family("bundle.7z.001"), family("bundle.7z.002"));
        assert_ne!(family("bundle.zip"), family("different.zip"));
    }
    #[test]
    fn archive_copy_projection_reuses_verified_file_identity_without_snapshot_copy() {
        let (_temp, mut source, target, mut opts) = fixture("zip", "zip", false);
        source.selected = vec!["docs/a.txt".into()];
        opts.overwrite = "rename".into();
        let export = prepare(&source, &JobControl::new()).unwrap();
        let old_path = PathBuf::from(&export.roots()[0]);
        let before = directory_identity(&File::open(&old_path).unwrap()).unwrap();
        let prepared = archive_add::prepare_export(&target, export, "backup", &opts).unwrap();
        assert!(!old_path.exists());
        let after = directory_identity(&File::open(prepared.cwd.join("backup/a (2).txt")).unwrap())
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(prepared.names, vec!["backup/a (2).txt"]);
    }
}
