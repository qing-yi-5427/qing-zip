use super::*;
use std::time::{Duration, UNIX_EPOCH};

fn write_at(path: &Path, bytes: &[u8], seconds: u64) {
    fs::write(path, bytes).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)))
        .unwrap();
}

#[test]
fn update_mode_is_strict_and_old_requests_default_to_add() {
    let options: EngineOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(options.update_mode, UpdateMode::Add);
    for mode in ["add", "update", "fresh", "sync"] {
        let options: EngineOptions =
            serde_json::from_value(serde_json::json!({"update_mode":mode})).unwrap();
        assert_eq!(serde_json::to_value(options).unwrap()["update_mode"], mode);
    }
    for mode in ["-u!evil", "Sync", "", "update x"] {
        assert!(
            serde_json::from_value::<EngineOptions>(serde_json::json!({"update_mode":mode}))
                .is_err()
        );
    }
}

#[test]
fn update_modes_real_zip_and_solid_7z_follow_official_action_matrix() {
    for (format, password) in [
        ("zip", None),
        ("7z", None),
        ("zip", Some("outer-secret")),
        ("7z", Some("outer-secret")),
    ] {
        for mode in [
            UpdateMode::Add,
            UpdateMode::Update,
            UpdateMode::Fresh,
            UpdateMode::Sync,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let inputs = temp.path().join("input");
            fs::create_dir(&inputs).unwrap();
            let a = inputs.join("A");
            let b = inputs.join("B");
            fs::create_dir(&a).unwrap();
            fs::create_dir(&b).unwrap();
            let timestamp = 1_700_000_000;
            for name in ["newer", "older", "same", "size", "missing"] {
                write_at(&a.join(name), b"old", timestamp);
            }
            write_at(&b.join("missing"), b"old", timestamp);
            write_at(&b.join("kept"), b"old", timestamp);
            write_at(&inputs.join("outside"), b"outside", timestamp);
            let archive = temp.path().join(format!("archive.{format}"));
            let base = EngineOptions {
                password: password.map(str::to_owned),
                header_encryption: if format == "7z" && password.is_some() {
                    Some(true)
                } else {
                    None
                },
                solid: if format == "7z" { Some(true) } else { None },
                threads: Some(1),
                ..Default::default()
            };
            create(
                archive.to_str().unwrap(),
                &[
                    a.display().to_string(),
                    b.display().to_string(),
                    inputs.join("outside").display().to_string(),
                ],
                format,
                &base,
            )
            .unwrap();
            write_at(&a.join("newer"), b"new", timestamp + 60);
            write_at(&a.join("older"), b"older different size", timestamp - 60);
            write_at(&a.join("same"), b"NEW", timestamp);
            write_at(&a.join("size"), b"new size", timestamp);
            write_at(&a.join("added"), b"added", timestamp);
            fs::remove_file(a.join("missing")).unwrap();
            fs::remove_file(b.join("missing")).unwrap();
            let opts = EngineOptions {
                update_mode: mode,
                ..base
            };
            update(
                archive.to_str().unwrap(),
                "add",
                &[a.display().to_string(), b.display().to_string()],
                &opts,
            )
            .unwrap();
            test(archive.to_str().unwrap(), &opts).unwrap();
            if password.is_some() {
                let entries = list(archive.to_str().unwrap(), &opts).unwrap();
                assert!(entries.iter().filter(|entry| !entry.is_dir).all(|entry| {
                    entry.encrypted && (format != "zip" || entry.method.contains("AES-256"))
                }));
            }
            let dest = temp.path().join("output");
            extract(archive.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
            assert_eq!(fs::read(dest.join("A/newer")).unwrap(), b"new");
            assert_eq!(
                fs::read(dest.join("A/older")).unwrap(),
                if mode == UpdateMode::Add {
                    b"older different size".as_slice()
                } else {
                    b"old"
                }
            );
            assert_eq!(
                fs::read(dest.join("A/same")).unwrap(),
                if mode == UpdateMode::Add {
                    b"NEW"
                } else {
                    b"old"
                }
            );
            assert_eq!(fs::read(dest.join("A/size")).unwrap(), b"new size");
            assert_eq!(dest.join("A/added").exists(), mode != UpdateMode::Fresh);
            assert_eq!(dest.join("A/missing").exists(), mode != UpdateMode::Sync);
            assert_eq!(dest.join("B/missing").exists(), mode != UpdateMode::Sync);
            assert_eq!(fs::read(dest.join("outside")).unwrap(), b"outside");
        }
    }
}

#[test]
fn compression_update_rejects_a_readable_but_unencrypted_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sealed.zip");
    let source = dir.path().join("report.txt");
    fs::write(&source, b"confidential original").unwrap();
    let path = path.to_str().unwrap();
    let inputs = [source.display().to_string()];
    let opts = EngineOptions { password: Some("test-fixture-password".into()), ..Default::default() };
    create(path, &inputs, "zip", &opts).unwrap();
    let original = fs::read(path).unwrap();
    fs::write(&source, b"confidential replacement").unwrap();
    let wrong = dir.path().join("plain.zip");
    create(wrong.to_str().unwrap(), &inputs, "zip", &EngineOptions::default()).unwrap();
    let bytes = fs::read(wrong).unwrap();
    AFTER_ENGINE.with(|hook| *hook.borrow_mut() = Some(Box::new(move |candidate| fs::write(candidate, bytes).unwrap())));
    assert!(update(path, "add", &inputs, &opts).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}

thread_local! {
    pub(super) static AFTER_ENGINE: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> = const { std::cell::RefCell::new(None) };
}

#[test]
fn compression_update_verifies_existing_password_and_preserves_mixed_zip_encryption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.zip");
    let public = dir.path().join("public.txt");
    let secret = dir.path().join("secret.txt");
    let added = dir.path().join("added.txt");
    fs::write(&public, b"public content").unwrap();
    fs::write(&secret, b"old secret").unwrap();
    fs::write(&added, b"new encrypted file").unwrap();
    let path = path.to_str().unwrap();
    create(path, &[public.display().to_string()], "zip", &EngineOptions::default()).unwrap();
    let password = EngineOptions { password: Some("test-fixture-password".into()), encryption: Some("aes256".into()), ..Default::default() };
    update(path, "add", &[secret.display().to_string()], &password).unwrap();
    let original = fs::read(path).unwrap();
    fs::write(&secret, b"replacement secret").unwrap();
    let inputs = [secret.display().to_string(), added.display().to_string()];
    for bad in [
        EngineOptions::default(),
        EngineOptions { password: Some("incorrect-test-password".into()), ..Default::default() },
        EngineOptions { encryption: Some("ZipCrypto".into()), ..password.clone() },
    ] {
        assert!(update(path, "add", &inputs, &bad).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }
    let options = EngineOptions { encryption: None, ..password };
    update(path, "add", &inputs, &options).unwrap();
    let listing = list(path, &options).unwrap();
    assert_eq!(listing.iter().find(|entry| entry.name == "public.txt").unwrap().encrypted, false);
    for name in ["secret.txt", "added.txt"] {
        let entry = listing.iter().find(|entry| entry.name == name).unwrap();
        assert!(entry.encrypted && entry.method.contains("AES-256"), "{entry:?}");
    }
    assert_eq!(preview(path, "secret.txt", &options).unwrap(), b"replacement secret");
}

#[cfg(windows)]
#[test]
fn compression_update_projects_roots_under_the_chosen_folder_for_each_mode() {
    for mode in [UpdateMode::Add, UpdateMode::Update, UpdateMode::Fresh, UpdateMode::Sync] {
        let dir = tempfile::tempdir().unwrap();
        let archived = dir.path().join("target");
        let original = archived.join("chosen");
        let local = dir.path().join("input/chosen");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(&local).unwrap();
        let time = 1_700_000_000;
        write_at(&original.join("replace.txt"), b"old", time);
        write_at(&original.join("missing.txt"), b"old", time);
        write_at(&archived.join("keep.txt"), b"keep", time);
        write_at(&local.join("replace.txt"), b"new", time + 120);
        write_at(&local.join("new.txt"), b"new", time + 120);
        let path = dir.path().join("nested.zip");
        let path = path.to_str().unwrap();
        create(path, &[archived.display().to_string()], "zip", &EngineOptions::default()).unwrap();
        let opts = EngineOptions { update_mode: mode, ..Default::default() };
        update_in_folder(path, &[local.display().to_string()], "target", &opts).unwrap();
        let set: HashSet<_> = list(path, &opts).unwrap().into_iter().map(|entry| entry.name.replace('\\', "/")).collect();
        assert!(set.contains("target/keep.txt"), "{set:?}");
        assert!(!set.contains("chosen/replace.txt"), "{set:?}");
        assert_eq!(set.contains("target/chosen/new.txt"), mode != UpdateMode::Fresh, "{set:?}");
        assert_eq!(set.contains("target/chosen/missing.txt"), mode != UpdateMode::Sync, "{set:?}");
        assert_eq!(preview(path, "target/chosen/replace.txt", &opts).unwrap(), b"new");
    }
}

#[test]
fn sync_failed_verification_and_cancel_after_engine_keep_original_archive() {
    for cancel in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("old.txt"), b"old").unwrap();
        let path = temp.path().join("a.zip");
        create(
            path.to_str().unwrap(),
            &[input.display().to_string()],
            "zip",
            &EngineOptions::default(),
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        fs::remove_file(input.join("old.txt")).unwrap();
        fs::write(input.join("new.txt"), b"new").unwrap();
        let ctl = JobControl::new();
        let c = ctl.clone();
        AFTER_ENGINE.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |staged| {
                if cancel {
                    c.cancel();
                } else {
                    fs::write(staged, b"corrupt proposal").unwrap();
                }
            }))
        });
        let opts = EngineOptions {
            update_mode: UpdateMode::Sync,
            control: Some(ctl),
            ..Default::default()
        };
        assert!(update(
            path.to_str().unwrap(),
            "add",
            &[input.display().to_string()],
            &opts
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn update_modes_do_not_leak_to_unrelated_operations_or_delete_commands() {
    let request = crate::operations::Request {
        operation: "create".into(),
        options: EngineOptions {
            update_mode: UpdateMode::Sync,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(crate::operations::execute_request(request).is_err());
    assert!(update(
        "missing.zip",
        "delete",
        &["a".into()],
        &EngineOptions {
            update_mode: UpdateMode::Sync,
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("更新模式"));
}
