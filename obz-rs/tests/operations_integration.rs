//! Exercise the shipped CLI JSON dispatcher, not only individual core methods.
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, process::{Command, Stdio}};

fn call(root: &Path, request: Value) -> Result<Value, String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_obz"))
        .arg("json").env("LOCALAPPDATA", root.join("profile"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(serde_json::to_string(&request).unwrap().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
    serde_json::from_slice(&output.stdout).map_err(|e|e.to_string())
}

#[test]
fn cli_recovery_finds_only_valid_encrypted_passwords() {
    let t = tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("secret.txt"); fs::write(&input,b"encrypted fixture").unwrap();
    let archive=p.join("secret.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[input],"options":{"password":"ba"}})).unwrap();
    let found=call(p,json!({"operation":"recover","path":archive,"candidates":["wrong","ba"],"max_attempts":3})).unwrap();
    assert_eq!(found["status"],"found"); assert_eq!(found["password"],"ba"); assert_eq!(found["attempts"],2);
    let exhausted=call(p,json!({"operation":"recover","path":archive,"candidates":["wrong"],"max_attempts":2})).unwrap();
    assert_eq!(exhausted["status"],"exhausted"); assert!(exhausted["password"].is_null());
    let charset=call(p,json!({"operation":"recover","path":archive,"charset":"ab","min_length":2,"max_length":2,"max_attempts":4})).unwrap();
    assert_eq!(charset["password"],"ba"); assert_eq!(charset["attempts"],3);
    let plain=p.join("plain.zip"); call(p,json!({"operation":"create","path":plain,"inputs":[input]})).unwrap();
    assert!(call(p,json!({"operation":"recover","path":plain,"candidates":["anything"]})).unwrap_err().contains("no encrypted"));
}

#[test]
fn cli_smart_and_selected_extraction_are_real() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let a=p.join("a.txt"); let b=p.join("b.txt");
    fs::write(&a,b"a").unwrap(); fs::write(&b,b"b").unwrap(); let archive=p.join("bundle.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[a,b]})).unwrap();
    let dest=p.join("out"); let result=call(p,json!({"operation":"extract","path":archive,"dest":dest,"smart":true})).unwrap();
    assert_eq!(result["files"],2); assert_eq!(fs::read(dest.join("bundle/a.txt")).unwrap(),b"a");
    let selected=p.join("selected"); call(p,json!({"operation":"extract","path":archive,"dest":selected,"options":{"selected":["b.txt"]}})).unwrap();
    assert_eq!(fs::read(selected.join("b.txt")).unwrap(),b"b"); assert!(!selected.join("a.txt").exists());
}

fn write_zip(path: &Path, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for &(name, content) in files {
        archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        archive.write_all(content).unwrap();
    }
    archive.finish().unwrap();
    fs::read(path).unwrap()
}

#[test]
fn cli_smart_extraction_reports_the_retained_top_level_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let archive = root.join("bundle.zip");
    let original = write_zip(&archive, &[("BUNDLE/photos/a.txt", b"photo"), ("loose.txt", b"loose")]);
    let dest = root.join("selected");
    let response = call(root, json!({
        "operation": "extract", "path": archive, "dest": dest, "smart": true,
        "options": {"selected": ["BUNDLE"]},
    })).unwrap();
    assert_eq!(response["destination"], json!(dest));
    assert_eq!(response["output_directory"], json!(dest.join("BUNDLE")));
    assert_eq!(fs::read(dest.join("BUNDLE/photos/a.txt")).unwrap(), b"photo");
    assert!(!dest.join("loose.txt").exists());

    let dest = root.join("whole");
    let response = call(root, json!({
        "operation": "extract", "path": archive, "dest": dest, "smart": true,
    })).unwrap();
    assert_eq!(response["destination"], json!(dest.join("bundle")));
    assert!(response.get("output_directory").is_none());
    assert_eq!(fs::read(dest.join("bundle/loose.txt")).unwrap(), b"loose");
    assert_eq!(fs::read(archive).unwrap(), original);

    let archive = root.join("bundle.tar");
    let mut tar = tar::Builder::new(fs::File::create(&archive).unwrap());
    let mut header = tar::Header::new_gnu();
    header.set_size(5);
    header.set_mode(0o644);
    tar.append_data(&mut header, "BUNDLE/photo.txt", b"photo".as_slice()).unwrap();
    drop(tar.into_inner().unwrap());
    let dest = root.join("tar");
    let response = call(root, json!({
        "operation": "extract", "path": archive, "dest": dest, "smart": true,
    })).unwrap();
    assert_eq!(response["destination"], json!(dest));
    assert_eq!(response["output_directory"], json!(dest.join("BUNDLE")));
    assert_eq!(fs::read(dest.join("BUNDLE/photo.txt")).unwrap(), b"photo");
}

#[test]
fn cli_auto_rename_reserves_actual_extraction_paths() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let archive = root.join("album.zip");
    let original = write_zip(&archive, &[
        ("photo.txt", b"base photo"),
        ("PHOTO (1).txt", b"numbered photo"),
        ("photo (2).txt/cover.txt", b"cover"),
    ]);

    for (label, selected, renamed, count) in [
        ("whole", Vec::new(), "photo (3).txt", 3),
        ("selected", vec!["photo.txt", "photo (2).txt"], "photo (1).txt", 2),
    ] {
        let output = root.join(label);
        fs::create_dir(&output).unwrap();
        fs::write(output.join("photo.txt"), b"local photo").unwrap();
        let response = call(root, json!({
            "operation": "extract", "path": archive, "dest": output,
            "options": {"overwrite": "rename", "selected": selected},
        })).unwrap();
        assert_eq!(response["files"], count);
        assert_eq!(fs::read(output.join("photo.txt")).unwrap(), b"local photo");
        assert_eq!(fs::read(output.join(renamed)).unwrap(), b"base photo");
        assert_eq!(fs::read(output.join("photo (2).txt/cover.txt")).unwrap(), b"cover");
        if label == "whole" {
            assert_eq!(fs::read(output.join("PHOTO (1).txt")).unwrap(), b"numbered photo");
        }
        assert_eq!(fs::read_dir(&output).unwrap().count(), count as usize + 1);
    }
    assert_eq!(fs::read(&archive).unwrap(), original);
}

#[test]
fn cli_file_colliding_with_directory_honors_safe_conflict_policies() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let archive = root.join("album.zip");
    let original = write_zip(&archive, &[("photo.txt", b"photo"), ("notes.txt", b"notes")]);

    for policy in ["error", "overwrite", "rename", "skip"] {
        let output = root.join(policy);
        let occupied = output.join("photo.txt");
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("local.txt"), b"local").unwrap();
        let response = call(root, json!({
            "operation": "extract", "path": archive, "dest": output,
            "options": {"overwrite": policy},
        }));
        match policy {
            "rename" => {
                assert_eq!(response.unwrap()["files"], 2);
                assert_eq!(fs::read(output.join("photo (1).txt")).unwrap(), b"photo");
                assert_eq!(fs::read(output.join("notes.txt")).unwrap(), b"notes");
            }
            "skip" => {
                assert_eq!(response.unwrap()["files"], 1);
                assert!(!output.join("photo (1).txt").exists());
                assert_eq!(fs::read(output.join("notes.txt")).unwrap(), b"notes");
            }
            _ => {
                assert!(response.unwrap_err().contains("destination conflict"));
                assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
            }
        }
        assert_eq!(fs::read(occupied.join("local.txt")).unwrap(), b"local");
    }
    assert_eq!(fs::read(&archive).unwrap(), original);

    let nested = root.join("nested.zip");
    let original = write_zip(&nested, &[("notes.txt", b"notes"), ("folder/photo.txt", b"photo")]);
    let output = root.join("blocked-parent");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("folder"), b"local file").unwrap();
    for policy in ["rename", "skip"] {
        let error = call(root, json!({
            "operation": "extract", "path": nested, "dest": output,
            "options": {"overwrite": policy},
        })).unwrap_err();
        assert!(error.contains("output parent is a file") || error.contains("create output directory"), "{error}");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
        assert_eq!(fs::read(output.join("folder")).unwrap(), b"local file");
        assert_eq!(fs::read(&nested).unwrap(), original);
    }
}

#[test]
fn cli_preview_returns_images_but_not_active_content() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let image=p.join("pixel.gif"); let active=p.join("active.svg");
    fs::write(&image,b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00\x3b").unwrap();
    fs::write(&active,b"<svg onload='alert(1)'/>").unwrap(); let archive=p.join("images.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[image,active]})).unwrap();
    let result=call(p,json!({"operation":"preview","path":archive,"name":"pixel.gif"})).unwrap();
    assert_eq!(result["mime"],"image/gif"); assert!(result["data_url"].as_str().unwrap().starts_with("data:image/gif;base64,R0lG"));
    assert!(call(p,json!({"operation":"preview","path":archive,"name":"active.svg"})).is_err());
}

#[cfg(windows)]
#[test]
fn cli_vault_isolated_profile_integrates_password_id() {
    let t=tempfile::tempdir().unwrap(); let p=t.path();
    call(p,json!({"operation":"vault-put","name":"fixture","label":"Test","options":{"password":"vault-fixture-secret"}})).unwrap();
    let list=call(p,json!({"operation":"vault-list"})).unwrap(); assert_eq!(list[0]["id"],"fixture"); assert!(!list.to_string().contains("vault-fixture-secret"));
    let bytes=fs::read(p.join("profile/OpenBandiZip/passwords.dpapi")).unwrap(); assert!(!bytes.windows(20).any(|w|w==b"vault-fixture-secret"));
    let source=p.join("file.txt"); fs::write(&source,b"secret content").unwrap(); let archive=p.join("vault.7z");
    call(p,json!({"operation":"create","path":archive,"inputs":[source],"password_id":"fixture"})).unwrap();
    assert_eq!(call(p,json!({"operation":"test","path":archive,"password_id":"fixture"})).unwrap()["valid"],true);
    assert!(call(p,json!({"operation":"test","path":archive,"options":{"password":"wrong"}})).is_err());
    let vault = obz_core::services::Vault::open(p.join("profile/OpenBandiZip/passwords.dpapi")).unwrap();
    let manual = vault.save_password_book_entry("Test 2", "vault-fixture-secret").unwrap();
    vault.remember_extraction_passwords([("vault.7z", "another-secret")]).unwrap();
    let automatic = vault.extraction_passwords().unwrap().remove(0);
    let second_archive = p.join("automatic.7z");
    call(p, json!({"operation":"create","path":second_archive,"inputs":[source],"options":{"password":"another-secret"}})).unwrap();
    for (path, id, output) in [
        (&archive, manual.id.as_str(), "manual-out"),
        (&second_archive, automatic.id.as_str(), "automatic-out"),
    ] {
        let dest = p.join(output);
        call(p, json!({"operation":"extract","path":path,"dest":dest,"password_id":id})).unwrap();
        assert_eq!(fs::read(dest.join("file.txt")).unwrap(), b"secret content");
    }
    call(p,json!({"operation":"vault-delete","name":"fixture"})).unwrap();
    assert_eq!(call(p,json!({"operation":"vault-list"})).unwrap()[0]["id"], manual.id);
    call(p,json!({"operation":"vault-delete","name":manual.id})).unwrap();
    assert_eq!(call(p,json!({"operation":"vault-list"})).unwrap(),json!([]));
    assert_eq!(vault.password_book_password(&automatic.id).unwrap(), "another-secret");
}

#[cfg(windows)]
#[test]
fn cli_iso_creation_dispatch_roundtrips() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("中文.txt"); fs::write(&input,b"ISO payload").unwrap(); let archive=p.join("image.iso");
    let result=call(p,json!({"operation":"create","path":archive,"inputs":[input],"format":"iso"})).unwrap(); assert!(result["outputs"][0].is_string());
    let result=call(p,json!({"operation":"list","path":archive})).unwrap(); assert!(result["entries"].as_array().unwrap().iter().any(|e|e["name"]=="中文.txt"));
    let dest=p.join("out"); call(p,json!({"operation":"extract","path":archive,"dest":dest})).unwrap(); assert_eq!(fs::read(dest.join("中文.txt")).unwrap(),b"ISO payload");
}

#[test]
fn cli_repair_dispatch_preserves_original() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("data.txt"); fs::write(&input,b"recover me").unwrap(); let archive=p.join("broken.zip");
    obz_core::zipfmt::create_zip(archive.to_str().unwrap(),&[input.to_string_lossy().into_owned()],false).unwrap();
    let mut broken=fs::read(&archive).unwrap(); let cd=broken.windows(4).position(|w|w==b"PK\x01\x02").unwrap(); broken.truncate(cd); fs::write(&archive,&broken).unwrap();
    let fixed=p.join("fixed.zip"); let report=call(p,json!({"operation":"repair","path":archive,"dest":fixed})).unwrap();
    assert_eq!(report["recovered"],json!(["data.txt"])); assert_eq!(report["complete"],false); assert_eq!(fs::read(archive).unwrap(),broken);
    assert_eq!(call(p,json!({"operation":"test","path":fixed})).unwrap()["valid"],true);
}

#[cfg(windows)]
#[test]
fn cli_classic_stored_rar_volumes_are_verified_before_publication() {
    const VOLUMES: [(&str, &[u8]); 3] = [
        ("split-store.rar", include_bytes!("fixtures/deep/rar15/split-store.rar")),
        ("split-store.r00", include_bytes!("fixtures/deep/rar15/split-store.r00")),
        ("split-store.r01", include_bytes!("fixtures/deep/rar15/split-store.r01")),
    ];
    const PAYLOAD: &[u8] = b"rar15 oracle payload\n";
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let archive = root.join(VOLUMES[0].0);
    for (name, bytes) in VOLUMES {
        fs::write(root.join(name), bytes).unwrap();
    }
    let listing = call(root, json!({"operation":"list", "path":archive})).unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "payload.txt");
    assert_eq!(entries[0]["size"], PAYLOAD.len());
    assert_eq!(entries[0]["crc"], "C945A8B6");
    assert_eq!(call(root, json!({"operation":"test", "path":archive})).unwrap()["valid"], true);
    for (directory, selected) in [("full", vec![]), ("selected", vec!["payload.txt"])] {
        let destination = root.join(directory);
        let result = call(root, json!({"operation":"extract", "path":archive, "dest":destination,
            "options":{"selected":selected}})).unwrap();
        assert_eq!(result["files"], 1);
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
        assert_eq!(fs::read(destination.join("payload.txt")).unwrap(), PAYLOAD);
    }
    for (name, bytes) in VOLUMES {
        assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
    }

    // Add a normal independent member at the end of a copy of the last volume
    // to verify that a selected extraction leaves other verified members out.
    let extra_archive = rars::rar15_40::write_stored_archive(
        &[rars::rar15_40::StoredEntry {
            name: b"notes.txt",
            data: b"another ordinary member\n",
            file_time: 0,
            file_attr: 0x20,
            host_os: 3,
            password: None,
            file_comment: None,
        }],
        rars::rar15_40::WriterOptions::new(rars::ArchiveVersion::Rar15, rars::FeatureSet::store_only()),
    ).unwrap();
    let parsed = rars::ArchiveReader::read(&extra_archive).unwrap();
    let extra = parsed.as_rar15_40().unwrap().files().next().unwrap();
    let two_members = root.join("two-members");
    fs::create_dir(&two_members).unwrap();
    for (name, bytes) in VOLUMES {
        fs::write(two_members.join(name), bytes).unwrap();
    }
    let mut last = VOLUMES[2].1.to_vec();
    last.extend_from_slice(&extra_archive[extra.block.offset..extra.packed_range.end]);
    fs::write(two_members.join(VOLUMES[2].0), &last).unwrap();
    let two_archive = two_members.join(VOLUMES[0].0);
    let two_list = call(root, json!({"operation":"list", "path":two_archive})).unwrap();
    assert_eq!(two_list["entries"].as_array().unwrap().len(), 2);
    let picked = root.join("picked");
    let picked_result = call(root, json!({"operation":"extract", "path":two_archive, "dest":picked,
        "options":{"selected":["notes.txt"]}})).unwrap();
    assert_eq!(picked_result["files"], 1);
    assert_eq!(fs::read(picked.join("notes.txt")).unwrap(), b"another ordinary member\n");
    assert!(!picked.join("payload.txt").exists());

    let last_volume = root.join(VOLUMES[2].0);
    fs::rename(&last_volume, root.join("last.saved")).unwrap();
    let absent = root.join("missing-volume-output");
    assert!(call(root, json!({"operation":"test", "path":archive})).is_err());
    assert!(call(root, json!({"operation":"extract", "path":archive, "dest":absent})).is_err());
    assert!(!absent.exists());
    fs::rename(root.join("last.saved"), &last_volume).unwrap();

    let middle_volume = root.join(VOLUMES[1].0);
    let mut damaged_header = VOLUMES[1].1.to_vec();
    damaged_header[7] ^= 1; // The main header's stored checksum no longer matches.
    fs::write(&middle_volume, &damaged_header).unwrap();
    let bad_header = call(root, json!({"operation":"test", "path":archive})).unwrap_err();
    assert!(bad_header.contains("CRC"), "{bad_header}");
    assert_eq!(fs::read(&middle_volume).unwrap(), damaged_header);

    let mut changed = VOLUMES[1].1.to_vec();
    *changed.last_mut().unwrap() = b'X'; // Ordinary data damage, while the original headers stay intact.
    fs::write(&middle_volume, &changed).unwrap();
    let existing = root.join("existing-output");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("payload.txt"), b"existing output").unwrap();
    let test_error = call(root, json!({"operation":"test", "path":archive})).unwrap_err();
    assert!(test_error.contains("CRC"), "{test_error}");
    let extract_error = call(root, json!({"operation":"extract", "path":archive, "dest":existing,
        "options":{"overwrite":"overwrite", "selected":["payload.txt"]}})).unwrap_err();
    assert!(extract_error.contains("CRC"), "{extract_error}");
    assert_eq!(fs::read_dir(&existing).unwrap().count(), 1);
    assert_eq!(fs::read(existing.join("payload.txt")).unwrap(), b"existing output");
    assert_eq!(fs::read(&middle_volume).unwrap(), changed);
    assert_eq!(fs::read(&archive).unwrap(), VOLUMES[0].1);
    assert_eq!(fs::read(&last_volume).unwrap(), VOLUMES[2].1);
}
