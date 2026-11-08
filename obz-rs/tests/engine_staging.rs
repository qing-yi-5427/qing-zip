use obz_core::engine::{self, EngineOptions};
use std::{fs, path::Path};
fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
#[test]
fn solid_many_members_and_selected_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("solid");
    fs::create_dir(&src).unwrap();
    let bytes = "solid archive 中文 repeated data\n".repeat(4096);
    for i in 0..80 {
        fs::write(src.join(format!("{i:03}.txt")), &bytes).unwrap();
    }
    let archive = p(&t.path().join("solid.7z"));
    engine::create(&archive, &[p(&src)], "7z", &EngineOptions::default()).unwrap();
    let output = t.path().join("all");
    assert_eq!(
        engine::extract(&archive, &p(&output), &EngineOptions::default()).unwrap(),
        80
    );
    for i in 0..80 {
        assert_eq!(
            fs::read(output.join(format!("solid/{i:03}.txt"))).unwrap(),
            bytes.as_bytes()
        );
    }
    let chosen = EngineOptions {
        selected: vec!["solid/079.txt".into()],
        ..Default::default()
    };
    let output = t.path().join("selected");
    assert_eq!(engine::extract(&archive, &p(&output), &chosen).unwrap(), 1);
    assert!(!output.join("solid/000.txt").exists());
    assert_eq!(
        fs::read(output.join("solid/079.txt")).unwrap(),
        bytes.as_bytes()
    );
}
#[test]
fn anonymous_bzip2_stream_is_not_empty() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("source.txt");
    fs::write(&src, b"anonymous stream contents").unwrap();
    let archive = p(&t.path().join("stream.bz2"));
    engine::create(&archive, &[p(&src)], "bzip2", &EngineOptions::default()).unwrap();
    let entries = engine::list(&archive, &EngineOptions::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].size, 25);
    let output = t.path().join("out");
    assert_eq!(
        engine::extract(&archive, &p(&output), &EngineOptions::default()).unwrap(),
        1
    );
    assert_eq!(
        fs::read(output.join(&entries[0].name)).unwrap(),
        b"anonymous stream contents"
    );
}

#[test]
fn cancel_decode_does_not_publish_destination() {
    use obz_core::engine::JobControl;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("large");
    fs::create_dir(&src).unwrap();
    let mut block = vec![0u8; 16 * 1024 * 1024];
    let mut seed = 0x12345678u32;
    for b in &mut block {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *b = seed as u8;
    }
    fs::write(src.join("a"), &block).unwrap();
    fs::write(src.join("b"), &block).unwrap();
    let archive = p(&t.path().join("cancel.7z"));
    engine::create(
        &archive,
        &[p(&src)],
        "7z",
        &EngineOptions {
            level: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    let destination = t.path().join("out");
    let dest = p(&destination);
    let ctl = JobControl::new();
    let child_ctl = ctl.clone();
    let worker = thread::spawn(move || {
        engine::extract_with_control(&archive, &dest, &EngineOptions::default(), &child_ctl)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while ctl.snapshot().phase != "decoding" && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(ctl.snapshot().phase, "decoding");
    ctl.cancel();
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(!destination.exists());
}

#[cfg(nana_selected_worker)]
fn extract_split(archive: &Path, dest: &Path, selected: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_obz"));
    command
        .env("OPEN_ARCHIVE_PROFILE", "1")
        .arg("extract")
        .arg(archive)
        .arg(dest);
    for name in selected {
        command.args(["--include", name]);
    }
    command.output().unwrap()
}

#[cfg(nana_selected_worker)]
fn require_worker(result: &std::process::Output, success: bool, fields: &[&str]) {
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.success(), success, "exit={} {diagnostics}", result.status);
    let mut lines = diagnostics
        .lines()
        .filter(|l| l.starts_with("OA_SELECTED_WORKER "));
    let receipt = lines.next().expect(&diagnostics);
    assert!(lines.next().is_none(), "{diagnostics}");
    for field in fields {
        assert!(
            receipt.split_whitespace().any(|word| word == *field),
            "{receipt}"
        );
    }
}

#[cfg(nana_selected_worker)]
#[test]
fn split_lzma2_verifies_all_blocks_and_rejects_unselected_crc() {
    verify_split_lzma_method("LZMA2");
}

#[cfg(nana_selected_worker)]
#[test]
fn split_lzma_verifies_all_blocks_and_rejects_unselected_crc() {
    verify_split_lzma_method("LZMA");
}

#[cfg(nana_selected_worker)]
fn verify_split_lzma_method(method: &str) {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir_all(input.join("empty-dir")).unwrap();
    let keep = b"selected data\n".repeat(1000);
    let middle = b"second selected item\n".repeat(1000);
    let sibling = b"not selected\n".repeat(1000);
    let mut state = 12345u32;
    let discarded: Vec<u8> = (0..64 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    for (name, data) in [
        ("01.bin", &keep),
        ("02.bin", &sibling),
        ("03.bin", &middle),
        ("04.bin", &discarded),
    ] {
        fs::write(input.join(name), data).unwrap();
    }
    for solid in ["off", "on", "2f"] {
        let run = temp.path().join(solid);
        fs::create_dir(&run).unwrap();
        let archive = run.join("fixture.7z");
        let created = std::process::Command::new(engine::executable().unwrap())
            .current_dir(&input)
            .args(["a", "-t7z", "-mmt=1", "-mhc=off"])
            .arg(format!("-m0={method}"))
            .arg(format!("-ms={solid}"))
            .arg(&archive)
            .arg(".")
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let mut bytes = fs::read(archive).unwrap();
        let volume = |index: usize| run.join(format!("test.7z.{:03}", index + 1));
        let split = |data: &[u8]| {
            for (index, chunk) in data.chunks(4096).enumerate() {
                fs::write(volume(index), chunk).unwrap();
            }
        };
        split(&bytes);
        let dest = run.join("valid");
        let selected = ["01.bin", "03.bin", "empty-dir"];
        let ok = extract_split(&volume(0), &dest, &selected);
        require_worker(
            &ok,
            true,
            &[
                "status=ok",
                "completed=5",
                &format!("selected={}", keep.len() + middle.len()),
                &format!("discarded={}", sibling.len() + discarded.len()),
            ],
        );
        assert_eq!(fs::read(dest.join("01.bin")).unwrap(), keep);
        assert_eq!(fs::read(dest.join("03.bin")).unwrap(), middle);
        assert!(dest.join("empty-dir").is_dir());
        assert_eq!(fs::read_dir(dest).unwrap().count(), 3);

        // -mhc=off gives this test a plain NextHeader. Change only the known
        // unselected item's CRC and repair the envelope CRCs. All packed bytes
        // remain valid, so detecting this failure requires checking that item.
        let header =
            32 + usize::try_from(u64::from_le_bytes(bytes[12..20].try_into().unwrap())).unwrap();
        assert_eq!(bytes[header], 1);
        let crc = crc32fast::hash(&discarded).to_le_bytes();
        let mut matches = bytes[header..]
            .windows(4)
            .enumerate()
            .filter(|(_, w)| *w == crc);
        let offset = matches.next().unwrap().0;
        assert!(matches.next().is_none());
        bytes[header + offset] ^= 1;
        let next_crc = crc32fast::hash(&bytes[header..]).to_le_bytes();
        bytes[28..32].copy_from_slice(&next_crc);
        let start_crc = crc32fast::hash(&bytes[12..32]).to_le_bytes();
        bytes[8..12].copy_from_slice(&start_crc);
        split(&bytes);
        let dest = run.join("corrupt");
        let rejected = extract_split(&volume(0), &dest, &selected);
        require_worker(
            &rejected,
            false,
            &["status=error", "code=3", "item_errors=1"],
        );
        assert!(!dest.exists());
    }
}

#[cfg(nana_selected_worker)]
#[test]
fn split_copy_cli_discards_unselected_bytes_and_rejects_their_crc_failure() {
    use std::process::Command;

    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    let keep = b"selected bytes";
    let discarded = b"independent unselected Copy payload!".repeat(100);
    fs::write(input.join("keep.txt"), keep).unwrap();
    fs::write(input.join("other.bin"), &discarded).unwrap();

    // Build the Copy archive from test-owned bytes, then partition it into
    // standard numeric volumes without relying on private fixtures or a decoder.
    let archive = temp.path().join("source.7z");
    let created = Command::new(engine::executable().unwrap())
        .current_dir(&input)
        .args(["a", "-t7z", "-mx=0", "-ms=off"])
        .arg(&archive)
        .args(["keep.txt", "other.bin"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{:?}", created.stderr);
    let bytes = fs::read(archive).unwrap();
    let positions: Vec<_> = bytes
        .windows(discarded.len())
        .enumerate()
        .filter_map(|(index, window)| (window == discarded.as_slice()).then_some(index))
        .collect();
    assert_eq!(
        positions.len(),
        1,
        "unselected Copy payload should occur once"
    );
    let volume_size = 1024;
    let volume = |number: usize| temp.path().join(format!("sample.7z.{number:03}"));
    for (index, chunk) in bytes.chunks(volume_size).enumerate() {
        fs::write(volume(index + 1), chunk).unwrap();
    }

    let destination = temp.path().join("valid");
    let result = extract_split(&volume(1), &destination, &["keep.txt"]);
    require_worker(
        &result,
        true,
        &["status=ok", &format!("discarded={}", discarded.len())],
    );
    assert_eq!(fs::read(destination.join("keep.txt")).unwrap(), keep);
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);

    // Flip a byte solely inside the unselected file. Its declared CRC remains.
    let offset = positions[0] + discarded.len() / 2;
    let damaged = volume(offset / volume_size + 1);
    let mut chunk = fs::read(&damaged).unwrap();
    chunk[offset % volume_size] ^= 0x40;
    fs::write(damaged, chunk).unwrap();
    let destination = temp.path().join("corrupt");
    let result = extract_split(&volume(1), &destination, &["keep.txt"]);
    require_worker(&result, false, &["status=error", "code=3"]);
    assert!(!destination.exists());
}

#[cfg(nana_selected_worker)]
#[test]
fn selected_split_commits_only_verified_files_on_the_target_volume() {
    use std::process::Command;
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("ordinary");
    fs::create_dir(&parent).unwrap();
    set_ordinary_acl(&parent);

    let source = tmp.path().join("src");
    fs::create_dir_all(source.join("nested")).unwrap();
    let payload = b"selected data".repeat(16000);
    fs::write(source.join("nested/data.bin"), &payload).unwrap();
    let selected_file = source.join("nested/data.bin");
    let writable = fs::metadata(&selected_file).unwrap().permissions();
    let mut readonly = writable.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&selected_file, readonly).unwrap();
    let unselected = b"uniquely unselected payload".repeat(10000);
    fs::write(source.join("other.bin"), &unselected).unwrap();
    let archive = tmp.path().join("sample.7z");
    let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join(obz_core::engine::BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
    let made = Command::new(exe)
        .current_dir(&source)
        .args(["a", "-t7z", "-mx=0", "-ms=off", "-v32k"])
        .arg(&archive)
        .args(["nested", "other.bin"])
        .output()
        .unwrap();
    assert!(made.status.success());
    fs::set_permissions(&selected_file, writable.clone()).unwrap();
    let part = |n| tmp.path().join(format!("sample.7z.{n:03}"));
    let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    fs::write(format!("{}:Zone.Identifier", part(1).display()), zone).unwrap();
    let extract = |dest: &Path| {
        Command::new(env!("CARGO_BIN_EXE_obz"))
            .arg("extract")
            .arg(part(1))
            .arg(dest)
            .args(["--include", "nested/data.bin"])
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .output()
            .unwrap()
    };
    for existing in [false, true] {
        let dest = parent.join(if existing { "existing" } else { "new" });
        if existing {
            fs::create_dir(&dest).unwrap();
            fs::write(dest.join("sentinel"), b"keep").unwrap();
        }
        let out = extract(&dest);
        let log = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "exit={} {log}", out.status);
        assert!(log.contains("OA_SELECTED_WORKER status=ok"), "{log}");
        assert!(
            log.contains(if existing {
                "moved=1 copied=0"
            } else {
                "native_directory_claim=1"
            }),
            "{log}"
        );
        let file = dest.join("nested/data.bin");
        assert_eq!(fs::read(&file).unwrap(), payload);
        assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert_eq!(
            fs::read(format!("{}:Zone.Identifier", file.display())).unwrap(),
            zone
        );
        assert_eq!(
            fs::metadata(&file).unwrap().modified().unwrap(),
            fs::metadata(source.join("nested/data.bin"))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert!(!dest.join("other.bin").exists());
        assert!(!extract(&dest).status.success());
        fs::set_permissions(&file, writable.clone()).unwrap();
        if existing {
            assert_eq!(fs::read(dest.join("sentinel")).unwrap(), b"keep");
        }
    }
    let mut changed = false;
    for n in 1.. {
        if !part(n).is_file() {
            break;
        }
        let mut bytes = fs::read(part(n)).unwrap();
        if let Some(i) = bytes
            .windows(27)
            .position(|v| v == b"uniquely unselected payload")
        {
            bytes[i] ^= 1;
            fs::write(part(n), bytes).unwrap();
            changed = true;
            break;
        }
    }
    assert!(changed);
    let rejected = parent.join("rejected");
    let out = extract(&rejected);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("OA_SELECTED_WORKER status=error"));
    assert!(!rejected.exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 2);
    let reference = parent.join("control/nested");
    fs::create_dir_all(&reference).unwrap();
    fs::write(reference.join("data.bin"), []).unwrap();
    let acl = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            r#"$ErrorActionPreference='Stop'; $root=$env:QING_TEST_OUTPUT_PARENT; function ReadAcl($p) { $acl=if ([IO.Directory]::Exists($p)) { [IO.Directory]::GetAccessControl($p) } else { [IO.File]::GetAccessControl($p) }; $acl.GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]'Owner,Group,Access') }; foreach ($path in @('','nested','nested/data.bin')) { $want=ReadAcl ([IO.Path]::Combine($root,'control',$path)); foreach ($dest in @('new','existing')) { if ((ReadAcl ([IO.Path]::Combine($root,$dest,$path))) -ne $want) { throw 'published ACL differs from ordinary file creation' } } }"#])
        .env("QING_TEST_OUTPUT_PARENT", &parent).output().unwrap();
    assert!(
        acl.status.success(),
        "{}",
        String::from_utf8_lossy(&acl.stderr)
    );
}

#[cfg(nana_selected_worker)]
fn set_ordinary_acl(parent: &Path) {
    use std::process::Command;
    // Give this test directory a normal, auto-inherited DACL regardless of the
    // runner's TEMP policy. The readonly Users ACE exercises post-commit ACLs.
    let acl = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            r#"$ErrorActionPreference='Stop'; $s=New-Object System.Security.AccessControl.DirectorySecurity; $s.SetSecurityDescriptorSddlForm('D:PAI(A;OICI;FA;;;'+[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value+')(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)',[Security.AccessControl.AccessControlSections]::Access); [IO.Directory]::SetAccessControl($env:QING_TEST_OUTPUT_PARENT, $s)"#])
        .env("QING_TEST_OUTPUT_PARENT", parent).output().unwrap();
    assert!(
        acl.status.success(),
        "{}",
        String::from_utf8_lossy(&acl.stderr)
    );
}

#[cfg(nana_selected_worker)]
#[test]
fn bounded_zip_transaction_publishes_only_after_the_final_large_member_verifies() {
    use std::{fs::File, io::Write, process::Command};
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("targets");
    fs::create_dir(&parent).unwrap();
    set_ordinary_acl(&parent);
    let archive = tmp.path().join("mixed.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = stored.compression_method(zip::CompressionMethod::Deflated);
    for i in 0..140 {
        zip.start_file(format!("nested/{i:03}.txt"), deflated)
            .unwrap();
        zip.write_all(format!("input-{i}").repeat(32).as_bytes())
            .unwrap();
    }
    zip.add_directory("empty/", stored).unwrap();
    zip.start_file("zero", stored).unwrap();
    zip.start_file("large.bin", stored).unwrap();
    let marker = b"unique final stored member marker";
    let mut big = marker.to_vec();
    big.resize(512 * 1024, b'A');
    zip.write_all(&big).unwrap();
    zip.finish().unwrap();
    let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    fs::write(format!("{}:Zone.Identifier", archive.display()), zone).unwrap();
    let extract = |dest: &Path| {
        Command::new(env!("CARGO_BIN_EXE_obz"))
            .args(["extract"])
            .arg(&archive)
            .arg(dest)
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .output()
            .unwrap()
    };
    let good = parent.join("good");
    let out = extract(&good);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("bounded-zip-private-stage"));
    assert!(good.join("empty").is_dir() && fs::read(good.join("zero")).unwrap().is_empty());
    assert_eq!(fs::read(good.join("large.bin")).unwrap(), big);
    assert_eq!(
        fs::read(format!(
            "{}:Zone.Identifier",
            good.join("large.bin").display()
        ))
        .unwrap(),
        zone
    );
    for i in 0..140 {
        assert_eq!(
            fs::read(good.join(format!("nested/{i:03}.txt"))).unwrap(),
            format!("input-{i}").repeat(32).as_bytes()
        );
    }
    let mut data = fs::read(&archive).unwrap();
    let idx = data
        .windows(marker.len())
        .position(|s| s == marker)
        .unwrap();
    data[idx] ^= 1;
    fs::write(&archive, &data).unwrap();
    let failed = parent.join("bad");
    let out = extract(&failed);
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && log.contains("bounded-zip-private-stage"),
        "{log}"
    );
    assert!(!failed.exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
}

#[cfg(nana_selected_worker)]
#[test]
fn bounded_solid_7z_checks_the_whole_folder_before_publishing() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("targets");
    fs::create_dir(&parent).unwrap();
    set_ordinary_acl(&parent);
    let data = temp.path().join("source");
    fs::create_dir_all(data.join("empty-dir")).unwrap();
    fs::write(data.join("empty"), []).unwrap();
    for i in 0..132 {
        fs::write(
            data.join(format!("{i:03}.txt")),
            format!("file {i}\n").repeat(70),
        )
        .unwrap();
    }
    let utf = "\u{4e2d}\u{6587}.txt";
    let last = b"large distinct file".repeat(32_000);
    fs::write(data.join(utf), &last).unwrap();
    let nana = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join(engine::BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
    let archive = temp.path().join("space and \u{4e2d}\u{6587}.7z");
    let made = Command::new(nana)
        .current_dir(&data)
        .args([
            "a",
            "-t7z",
            "-m0=LZMA2:d=1m",
            "-ms=on",
            "-mhc=off",
            "-mmt=1",
        ])
        .arg(&archive)
        .arg(".")
        .output()
        .unwrap();
    assert!(made.status.success());
    let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    fs::write(format!("{}:Zone.Identifier", archive.display()), zone).unwrap();
    let extract = |dest: &Path| {
        Command::new(env!("CARGO_BIN_EXE_obz"))
            .arg("extract")
            .arg(&archive)
            .arg(dest)
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .output()
            .unwrap()
    };
    let good = parent.join("good");
    let valid = extract(&good);
    let log = String::from_utf8_lossy(&valid.stderr);
    assert!(
        valid.status.success() && log.contains("bounded-7z-private-stage"),
        "{log}"
    );
    assert!(good.join("empty-dir").is_dir() && fs::read(good.join("empty")).unwrap().is_empty());
    assert_eq!(fs::read(good.join(utf)).unwrap(), last);
    assert_eq!(
        fs::read(format!("{}:Zone.Identifier", good.join(utf).display())).unwrap(),
        zone
    );
    assert_eq!(
        fs::metadata(good.join(utf)).unwrap().modified().unwrap(),
        fs::metadata(data.join(utf)).unwrap().modified().unwrap()
    );
    for i in 0..132 {
        assert_eq!(
            fs::read(good.join(format!("{i:03}.txt"))).unwrap(),
            format!("file {i}\n").repeat(70).as_bytes()
        );
    }
    // Change one independently known substream CRC, preserving both envelope CRCs.
    let mut bytes = fs::read(&archive).unwrap();
    let off = 32 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let declared = crc32fast::hash(&last).to_le_bytes();
    let offsets: Vec<_> = bytes[off..]
        .windows(4)
        .enumerate()
        .filter_map(|(i, w)| (w == declared).then_some(i))
        .collect();
    assert_eq!(offsets.len(), 1);
    bytes[off + offsets[0]] ^= 1;
    let header = crc32fast::hash(&bytes[off..]);
    bytes[28..32].copy_from_slice(&header.to_le_bytes());
    let start = crc32fast::hash(&bytes[12..32]);
    bytes[8..12].copy_from_slice(&start.to_le_bytes());
    fs::write(&archive, bytes).unwrap();
    let bad = parent.join("bad");
    let failure = extract(&bad);
    assert!(
        !failure.status.success()
            && String::from_utf8_lossy(&failure.stderr).contains("bounded-7z-private-stage")
    );
    assert!(!bad.exists() && fs::read_dir(&parent).unwrap().count() == 1);
}


#[cfg(windows)]
#[test]
fn new_directory_rolls_back_published_files_and_preserves_foreign_data_on_cancel() {
    use std::{io::Write,thread,time::{Instant,Duration}};
    let temp=tempfile::tempdir().unwrap();
    let archive=temp.path().join("rollback.zip");
    let mut zip=zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for name in ["annotation/","attrs/"] { zip.add_directory(name,zip::write::SimpleFileOptions::default()).unwrap(); }
    for index in 0..2400 {
        zip.start_file(format!("data/file-{index:05}.txt"),zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)).unwrap();
        zip.write_all(b"owned contents").unwrap();
    }
    drop(zip.finish().unwrap());
    // TAR never qualifies for native ZIP/7z same-volume staging.
    let fallback=temp.path().join("fallback.tar");
    let fixture=temp.path().join("tar-input");
    zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap().extract(&fixture).unwrap();
    let inputs=["annotation", "attrs", "data"].map(|name|p(&fixture.join(name)));
    engine::create(&p(&fallback),&inputs,"tar",&EngineOptions::default()).unwrap();
    for (route,archive) in [("ordinary",&archive),("fallback",&fallback)] {
    for foreign in [false,true] {
        let dest=temp.path().join(format!("{route}-{foreign}"));
        let ctl=engine::JobControl::new();
        let worker=ctl.clone();
        let a=archive.clone();let target=dest.clone();
        let thread=thread::spawn(move || engine::extract(a.to_str().unwrap(),target.to_str().unwrap(),
            &EngineOptions{control:Some(worker),..EngineOptions::default()}));
        let leaf=dest.join("data/file-00000.txt");
        let deadline=Instant::now()+Duration::from_secs(15);
        while !leaf.is_file(){
            if thread.is_finished(){panic!("extraction ended before file publication: {:?}", thread.join());}
            assert!(Instant::now()<deadline, "file publication did not start");
            thread::sleep(Duration::from_millis(1));
        }
        if foreign {
            use std::{ffi::OsStr,os::windows::ffi::OsStrExt};
            #[link(name="kernel32")] extern "system" { fn SetFileAttributesW(file:*const u16,attrs:u32)->i32; }
            let name:Vec<u16>=OsStr::new(&dest.join("attrs")).encode_wide().chain(Some(0)).collect();
            assert_ne!(unsafe{SetFileAttributesW(name.as_ptr(),0x0001|0x0002)},0);
            fs::write(format!("{}:user-note",dest.join("annotation").display()),b"directory annotation").unwrap();
            fs::write(dest.join("external.txt"),b"outsider").unwrap();
            fs::write(format!("{}:foreign-note",leaf.display()),b"user annotation").unwrap();
        }
        ctl.cancel();
        let error=thread.join().unwrap().unwrap_err();
        assert!(error.contains("cancel"),"{error}");
        if foreign {
            assert!(error.contains("partial destination retained"),"{error}");
            use std::os::windows::fs::MetadataExt;
            assert_eq!(fs::metadata(dest.join("attrs")).unwrap().file_attributes()&3,3);
            assert_eq!(fs::read(format!("{}:user-note",dest.join("annotation").display())).unwrap(),b"directory annotation");
            assert_eq!(fs::read(dest.join("external.txt")).unwrap(),b"outsider");
            assert_eq!(fs::read(&leaf).unwrap(),b"owned contents");
            assert_eq!(fs::read(format!("{}:foreign-note",leaf.display())).unwrap(),b"user annotation");
            assert_eq!(fs::read_dir(dest.join("data")).unwrap().count(),1);
        } else {assert!(!dest.exists(),"{error}");}
    }
    }
}
