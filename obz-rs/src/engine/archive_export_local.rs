//! File-workspace-only export: preserve archive paths during validation, then
//! publish selected roots relative to the displayed archive directory.
use super::{archive_export, file_workspace, *};

fn normalized_key(path: &Path) -> String {
    let key = path_key(path);
    if let Some(rest) = key.strip_prefix("//?/unc/") {
        format!("//{rest}")
    } else {
        key.strip_prefix("//?/").unwrap_or(&key).to_owned()
    }
}

pub fn copy(
    path: &str,
    source: &archive_export::Source,
    destination: &str,
    opts: &EngineOptions,
) -> Result<file_workspace::Transfer, String> {
    if path.is_empty() || source.path.is_empty() || destination.is_empty() {
        return Err("请指定源压缩包和目标文件夹".into());
    }
    let ctl = control(opts);
    ctl.check()?;
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("无效的冲突处理方式".into());
    }
    let declared = absolute(path)?;
    let original = absolute(&source.path)?;
    no_links(&declared)?;
    no_links(&original)?;
    let declared = fs::canonicalize(declared).map_err(|e| e.to_string())?;
    let original = fs::canonicalize(original).map_err(|e| e.to_string())?;
    if normalized_key(&declared) != normalized_key(&original) {
        return Err("导出源与当前压缩包不一致".into());
    }
    let dest = absolute(destination)?;
    no_links(&dest)?;
    if !dest.is_dir() {
        return Err("目标文件夹不存在".into());
    }
    let dest = fs::canonicalize(dest).map_err(|e| e.to_string())?;
    // All archive validation/decryption is owned by the shared helper. It
    // retains original member paths and returns only selected, verified roots.
    let exported = archive_export::prepare(source, &ctl)?;
    for root in exported.roots() {
        let root = Path::new(root);
        let target = dest.join(root.file_name().ok_or("无效的导出项目")?);
        no_links(&target)?;
        let target = if target.try_exists().map_err(|e| e.to_string())? {
            fs::canonicalize(&target).map_err(|e| e.to_string())?
        } else {
            target
        };
        let target_key = normalized_key(&target);
        let overlaps_source = exported
            .source_paths()
            .iter()
            .chain(std::iter::once(&original))
            .any(|source| {
                let source_key = normalized_key(source);
                source_key == target_key || source_key.starts_with(&(target_key.clone() + "/"))
            });
        // Never replace the source archive through an exported root/subtree.
        // Unlike ordinary extraction this operation promises a read-only source.
        if overlaps_source {
            return Err("导出目标包含源压缩包，请选择其他文件夹".into());
        }
    }
    #[cfg(test)]
    AFTER_EXPORT.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(exported.roots());
        }
    });
    ctl.check()?;
    // Credentials and selected members belong exclusively to Source. Preserve
    // the local transfer's per-item atomic commits and honest partial errors.
    let local_options = EngineOptions {
        overwrite: opts.overwrite.clone(),
        control: Some(ctl),
        ..Default::default()
    };
    file_workspace::transfer(
        exported.roots(),
        dest.to_str().ok_or("无效的目标路径")?,
        false,
        &local_options,
    )
    // Export guard drops after transfer's handles, on success and on failure.
}

#[cfg(test)]
thread_local! {
    static AFTER_EXPORT: std::cell::RefCell<Option<Box<dyn FnOnce(&[String])>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(
        format: &str,
        password: Option<&str>,
    ) -> (tempfile::TempDir, archive_export::Source, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let docs = temp.path().join("input/docs");
        fs::create_dir_all(docs.join("sub/empty")).unwrap();
        fs::write(docs.join("a.txt"), b"new").unwrap();
        fs::write(docs.join("sub/b.txt"), b"child").unwrap();
        let archive = temp.path().join(format!("source.{format}"));
        create(
            archive.to_str().unwrap(),
            &[docs.display().to_string()],
            format,
            &EngineOptions {
                password: password.map(str::to_owned),
                ..Default::default()
            },
        )
        .unwrap();
        let dest = temp.path().join("output");
        fs::create_dir(&dest).unwrap();
        let source = archive_export::Source {
            path: archive.display().to_string(),
            folder: "docs/".into(),
            selected: vec!["docs/a.txt".into(), "docs/sub/".into()],
            password: password.map(str::to_owned),
            codepage: None,
        };
        (temp, source, dest)
    }
    #[test]
    fn archive_export_local_projects_roots_and_keeps_empty_directories_and_source() {
        for (format, password) in [("zip", None), ("7z", Some("source-secret"))] {
            let (_temp, source, dest) = fixture(format, password);
            let before = fs::read(&source.path).unwrap();
            let result = copy(
                &source.path,
                &source,
                dest.to_str().unwrap(),
                &EngineOptions {
                    password: Some("wrong-unused-target-password".into()),
                    selected: vec!["irrelevant".into()],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(result.copied, 2);
            assert_eq!(result.moved, 0);
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"new");
            assert_eq!(fs::read(dest.join("sub/b.txt")).unwrap(), b"child");
            assert!(dest.join("sub/empty").is_dir());
            assert!(!dest.join("docs").exists());
            assert_eq!(fs::read(&source.path).unwrap(), before);
        }
    }
    #[test]
    fn archive_export_local_preserves_conflict_policies() {
        for policy in ["error", "skip", "rename", "overwrite"] {
            let (_temp, mut source, dest) = fixture("zip", None);
            source.selected = vec!["docs/a.txt".into()];
            let original = fs::read(&source.path).unwrap();
            fs::write(dest.join("a.txt"), b"old").unwrap();
            let result = copy(
                &source.path,
                &source,
                dest.to_str().unwrap(),
                &EngineOptions {
                    overwrite: policy.into(),
                    ..Default::default()
                },
            );
            if policy == "error" {
                assert!(result.is_err());
            } else {
                assert!(result.is_ok(), "{policy}: {result:?}");
            }
            assert_eq!(
                fs::read(dest.join("a.txt")).unwrap(),
                if policy == "overwrite" {
                    b"new"
                } else {
                    b"old"
                }
            );
            if policy == "rename" {
                assert_eq!(fs::read(dest.join("a (1).txt")).unwrap(), b"new");
            }
            assert_eq!(fs::read(&source.path).unwrap(), original);
        }
    }
    #[test]
    fn archive_export_local_cancel_after_prepare_cleans_private_roots_without_publication() {
        let (_temp, source, dest) = fixture("zip", None);
        let before = fs::read(&source.path).unwrap();
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        let saved = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let capture = saved.clone();
        AFTER_EXPORT.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |roots| {
                *capture.borrow_mut() = roots.to_vec();
                cancel.cancel();
            }))
        });
        assert!(copy(
            &source.path,
            &source,
            dest.to_str().unwrap(),
            &EngineOptions {
                control: Some(ctl),
                ..Default::default()
            }
        )
        .is_err());
        assert!(!saved.borrow().is_empty());
        assert!(saved.borrow().iter().all(|p| !Path::new(p).exists()));
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        assert_eq!(fs::read(&source.path).unwrap(), before);
    }
    #[test]
    fn archive_export_local_rejects_mismatched_path_and_source_overwrite() {
        let (temp, mut source, dest) = fixture("zip", None);
        let other = temp.path().join("other.zip");
        fs::copy(&source.path, &other).unwrap();
        assert!(copy(
            other.to_str().unwrap(),
            &source,
            dest.to_str().unwrap(),
            &EngineOptions::default()
        )
        .unwrap_err()
        .contains("不一致"));
        let input = temp.path().join("source.zip");
        let archive = temp.path().join("outer.zip");
        create(
            archive.to_str().unwrap(),
            &[input.display().to_string()],
            "zip",
            &EngineOptions::default(),
        )
        .unwrap();
        // Put the source name in itself without needing a cyclic source file.
        update(
            archive.to_str().unwrap(),
            "rename",
            &["source.zip".into(), "outer.zip".into()],
            &EngineOptions::default(),
        )
        .unwrap();
        source.path = archive.display().to_string();
        source.folder.clear();
        source.selected = vec!["outer.zip".into()];
        let before = fs::read(&archive).unwrap();
        assert!(copy(
            &source.path,
            &source,
            temp.path().to_str().unwrap(),
            &EngineOptions {
                overwrite: "overwrite".into(),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("源压缩包"));
        assert_eq!(fs::read(archive).unwrap(), before);
    }
    #[test]
    fn archive_export_local_protects_adjacent_volume_names_across_verbatim_prefixes() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.zip");
        let sibling = temp.path().join("source.z01");
        fs::write(&sibling, b"must-preserve-volume").unwrap();
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("source.z01", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"replacement").unwrap();
        writer.finish().unwrap();
        let source = archive_export::Source {
            path: path.display().to_string(),
            selected: vec!["source.z01".into()],
            ..Default::default()
        };
        let before = fs::read(&path).unwrap();
        let error = copy(
            &source.path,
            &source,
            temp.path().to_str().unwrap(),
            &EngineOptions {
                overwrite: "overwrite".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("源压缩包"));
        assert_eq!(fs::read(&sibling).unwrap(), b"must-preserve-volume");
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn archive_export_local_expands_source_and_destination_short_names_before_family_guard() {
        use std::{io::Write, os::windows::ffi::OsStrExt};
        #[link(name = "kernel32")]
        extern "system" {
            fn GetShortPathNameW(long: *const u16, short: *mut u16, length: u32) -> u32;
        }
        fn short(path: &Path) -> PathBuf {
            let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let n = unsafe { GetShortPathNameW(input.as_ptr(), std::ptr::null_mut(), 0) };
            assert!(n > 0, "{}", std::io::Error::last_os_error());
            let mut output = vec![0u16; n as usize + 1];
            let size = unsafe {
                GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32)
            };
            assert!(size > 0);
            PathBuf::from(String::from_utf16_lossy(&output[..size as usize]))
        }
        let temp = tempfile::Builder::new()
            .prefix("qing archive family alias ")
            .tempdir()
            .unwrap();
        let source = temp.path().join("long archive filename.zip");
        let sibling = temp.path().join("long archive filename.z01");
        fs::write(&source, b"placeholder").unwrap();
        fs::write(&sibling, b"preserve-real-volume").unwrap();
        let source_short = short(&source);
        let sibling_short = short(&sibling);
        if source_short.file_name() == source.file_name()
            || sibling_short.file_name() == sibling.file_name()
        {
            eprintln!("8.3 aliases are disabled on this fixture volume; ordinary canonical/verbatim regression remains active");
            return;
        }
        let member = sibling_short.file_name().unwrap().to_str().unwrap();
        let mut z = zip::ZipWriter::new(File::create(&source).unwrap());
        z.start_file(member, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"replacement").unwrap();
        z.finish().unwrap();
        let before = fs::read(&source).unwrap();
        let selected = archive_export::Source {
            path: source_short.display().to_string(),
            selected: vec![member.to_owned()],
            ..Default::default()
        };
        let error = copy(
            &selected.path,
            &selected,
            temp.path().to_str().unwrap(),
            &EngineOptions {
                overwrite: "overwrite".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("源压缩包"));
        assert_eq!(fs::read(sibling).unwrap(), b"preserve-real-volume");
        assert_eq!(fs::read(source).unwrap(), before);
        eprintln!("8.3 source stem and destination member alias protection exercised successfully");
    }
}
