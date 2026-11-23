use obz_core::{
    engine::{self, deep_extract, EngineOptions, Entry},
    operations,
};
use serde::Serialize;
use std::path::{Component, Path, Prefix};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CompressionSource {
    File,
    Directory,
}

#[derive(Debug)]
enum Intake {
    Browse {
        path: String,
        entries: Vec<Entry>,
    },
    Deep {
        path: String,
    },
    Review {
        path: String,
        reason: String,
        compression_source: Option<CompressionSource>,
    },
}

fn review(path: String, reason: impl Into<String>) -> Intake {
    Intake::Review {
        path,
        reason: reason.into(),
        compression_source: None,
    }
}

#[tauri::command]
pub async fn inspect_home_archive(path: String) -> Result<tauri::ipc::Response, String> {
    #[derive(Serialize)]
    #[serde(tag = "route", rename_all = "snake_case")]
    enum Payload<'a> {
        Browse {
            path: &'a str,
            rows: crate::archive_listing::Listing<'a>,
        },
        Deep {
            path: &'a str,
        },
        Review {
            path: &'a str,
            reason: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            compression_source: Option<CompressionSource>,
        },
    }

    tauri::async_runtime::spawn_blocking(move || {
        let result = inspect(path);
        let payload = match &result {
            Intake::Browse { path, entries } => Payload::Browse {
                path,
                rows: crate::archive_listing::Listing(entries),
            },
            Intake::Deep { path } => Payload::Deep { path },
            Intake::Review {
                path,
                reason,
                compression_source,
            } => Payload::Review {
                path,
                reason,
                compression_source: *compression_source,
            },
        };
        crate::archive_listing::response(payload)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn pick_home_files() -> Vec<String> {
    rfd::FileDialog::new()
        .set_title("选择要识别的文件")
        .add_filter("所有文件", &["*"])
        .pick_files()
        .map(|paths| {
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn compression_source_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| engine::validate_name(name).is_ok())
}

fn inspect(path: String) -> Intake {
    let source = Path::new(&path);
    let ordinary_prefix = !matches!(source.components().next(),
        Some(Component::Prefix(prefix)) if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _)));
    if path.is_empty()
        || path.contains(['\0', '\r', '\n'])
        || path.contains("://")
        || !ordinary_prefix
    {
        return review(path, "无效的归档路径");
    }
    let path = match crate::instance::canonical_archive_path(path.clone()) {
        Ok(file) => file,
        Err(reason) => {
            if let Ok(directory) = crate::desktop::canonical_spelling(source, true) {
                if compression_source_name(Path::new(&directory)) {
                    return match std::fs::read_dir(&directory) {
                        Ok(_) => Intake::Review {
                            path: directory,
                            reason: "文件夹可以作为压缩来源。".into(),
                            compression_source: Some(CompressionSource::Directory),
                        },
                        Err(error) => review(directory, error.to_string()),
                    };
                }
            }
            return review(path, reason);
        }
    };
    let source = Path::new(&path);
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("");
    let application = deep_extract::home_application_extension(source);
    let volume = !application && deep_extract::home_volume_member(source);
    let named = application
        || crate::associations::file_dialog_extensions()
            .iter()
            .any(|e| ext.eq_ignore_ascii_case(e));
    let disguised = !named
        && !volume
        && match deep_extract::home_deep_candidate(source) {
            Ok(candidate) => candidate,
            Err(reason) => return review(path, reason),
        };
    let options = EngineOptions::default();
    match operations::list_archive_once(&path, &options) {
        Ok(entries) if application || deep_extract::application_container(&entries) => {
            Intake::Browse { path, entries }
        }
        Ok(entries)
            if disguised
                || volume
                || (deep_extract::home_nested_candidate(source, &entries)
                    && deep_extract::home_deep_candidate(source).unwrap_or(false)) =>
        {
            Intake::Deep { path }
        }
        Ok(entries) => Intake::Browse { path, entries },
        Err(reason) if volume || (disguised && requires_password(&reason)) => Intake::Deep { path },
        Err(_) if !named && !disguised => {
            let compression_source =
                compression_source_name(source).then_some(CompressionSource::File);
            Intake::Review {
                path,
                reason: "未识别为支持的压缩包。".into(),
                compression_source,
            }
        }
        Err(reason) => review(path, reason),
    }
}

fn requires_password(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    [
        "wrong password",
        "incorrect password",
        "enter password",
        "password required",
        "password is required",
        "需要密码",
    ]
    .iter()
    .any(|text| reason.contains(text))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn archive(
        root: &Path,
        name: &str,
        input: &Path,
        format: &str,
        options: &EngineOptions,
    ) -> PathBuf {
        let output = root.join(name);
        engine::create(
            output.to_str().unwrap(),
            &[input.to_str().unwrap().to_owned()],
            format,
            options,
        )
        .unwrap();
        output
    }

    #[test]
    fn identifies_browsing_deep_wrappers_and_protected_files_without_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let note = root.join("note.txt");
        fs::write(&note, b"ordinary payload").unwrap();
        let options = EngineOptions::default();
        let plain = archive(root, "plain.zip", &note, "zip", &options);
        let disguised = root.join("inner.dat");
        fs::copy(&plain, &disguised).unwrap();
        let extensionless = root.join("inner");
        fs::copy(&plain, &extensionless).unwrap();
        let named_wrapper = archive(root, "named.zip", &plain, "zip", &options);
        let hidden_wrapper = archive(root, "wrapper.zip", &disguised, "zip", &options);
        let doc = root.join("protected.docx");
        fs::copy(&named_wrapper, &doc).unwrap();
        let doc_wrapper = archive(root, "document-wrapper.zip", &doc, "zip", &options);
        let photo = root.join("photo.jpg");
        fs::write(&photo, b"\xff\xd8\xff\xe0JPEG sample\xff\xd9").unwrap();

        let files = [
            plain,
            disguised,
            extensionless,
            named_wrapper,
            hidden_wrapper,
            doc,
            doc_wrapper,
            note,
            photo,
        ];
        let before = files
            .iter()
            .map(fs::read)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (i, file) in files.iter().enumerate() {
            let result = inspect(file.to_str().unwrap().into());
            match (i, result) {
                (0 | 5 | 6, Intake::Browse { entries, .. }) => assert_eq!(entries.len(), 1),
                (1..=4, Intake::Deep { .. })
                | (
                    7 | 8,
                    Intake::Review {
                        compression_source: Some(CompressionSource::File),
                        ..
                    },
                ) => {}
                (_, unexpected) => panic!("{}: {unexpected:?}", file.display()),
            }
        }
        assert_eq!(
            before,
            files
                .iter()
                .map(fs::read)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
        assert_eq!(fs::read_dir(root).unwrap().count(), files.len());
    }

    #[test]
    fn only_valid_plain_inputs_offer_compression() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let plain = root.join("笔记.txt");
        let directory = root.join("资料 文件夹");
        let broken = root.join("broken.zip");
        let missing = root.join("missing.txt");
        let payload = b"plain test payload";
        fs::write(&plain, payload).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::write(&broken, payload).unwrap();
        for (path, expected) in [
            (&plain, Some(CompressionSource::File)),
            (&directory, Some(CompressionSource::Directory)),
            (&broken, None),
            (&missing, None),
        ] {
            match inspect(path.display().to_string()) {
                Intake::Review {
                    compression_source,
                    reason,
                    ..
                } => {
                    assert_eq!(compression_source, expected, "{}", path.display());
                    assert!(!reason.is_empty());
                }
                other => panic!("{}: {other:?}", path.display()),
            }
        }
        assert_eq!(fs::read(plain).unwrap(), payload);
        assert_eq!(fs::read(broken).unwrap(), payload);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root).unwrap().count(), 3);
    }

    #[test]
    fn identifies_password_requirements_and_multipart_family() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let input = root.join("note.txt");
        fs::write(&input, b"ordinary payload").unwrap();
        let protected = archive(
            root,
            "encrypted.7z",
            &input,
            "7z",
            &EngineOptions {
                password: Some("example-private-test-password".into()),
                header_encryption: Some(true),
                ..Default::default()
            },
        );
        let disguised = root.join("encrypted.dat");
        fs::copy(&protected, &disguised).unwrap();
        assert!(
            matches!(inspect(protected.display().to_string()), Intake::Review { reason, .. } if requires_password(&reason))
        );
        assert!(matches!(
            inspect(disguised.display().to_string()),
            Intake::Deep { .. }
        ));

        let first = root.join("family.7z.001");
        let second = root.join("family.7z.002");
        let archive = fs::read(&protected).unwrap();
        let (a, b) = archive.split_at(archive.len() / 2);
        fs::write(&first, a).unwrap();
        fs::write(&second, b).unwrap();
        for volume in [&first, &second] {
            assert!(matches!(
                inspect(volume.display().to_string()),
                Intake::Deep { .. }
            ));
        }
        assert_eq!(fs::read(first).unwrap(), a);
        assert_eq!(fs::read(second).unwrap(), b);
    }
}
