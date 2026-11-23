#![cfg(all(windows, target_pointer_width = "64", target_env = "msvc"))]

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

fn archive(path: &Path) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.add_directory("folder/", stored).unwrap();
    for (name, contents) in [
        ("folder/good.txt", b"independent good member".as_slice()),
        ("bad.txt", b"distinct content we can corrupt".as_slice()),
    ] {
        zip.start_file(name, stored).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
}

fn extract(source: &Path, destination: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_obz"))
        .env("OPEN_ARCHIVE_PROFILE", "1")
        .arg("extract")
        .arg(source)
        .arg(destination)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn verified_selection_and_existing_destination_use_direct_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input.zip");
    archive(&source);
    let original = fs::read(&source).unwrap();
    let destination = temp.path().join("existing");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("unrelated.txt"), b"preserve").unwrap();
    let result = extract(&source, &destination, &["--include", "folder/good.txt"]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success() && stderr.contains("OA_DIRECT_EXTRACT files=1 mode=2"),
        "{stderr}"
    );
    assert_eq!(
        fs::read(destination.join("folder/good.txt")).unwrap(),
        b"independent good member"
    );
    assert_eq!(
        fs::read(destination.join("unrelated.txt")).unwrap(),
        b"preserve"
    );
    assert!(!destination.join("bad.txt").exists());
    assert_eq!(fs::read(source).unwrap(), original);
}

fn stage_cancellation_helpers(root: &Path) {
    if let Some(portable) = std::env::var_os("QING_TEST_SELECTED_ZIP_PORTABLE") {
        let mut portable = zip::ZipArchive::new(fs::File::open(portable).unwrap()).unwrap();
        for index in 0..portable.len() {
            let mut file = portable.by_index(index).unwrap();
            let name = file.name().expect("Invalid portable member name");
            let required = matches!(name.as_ref(), "obz.exe" | "obz-worker.exe")
                || name.starts_with("runtime/nanazip/");
            if file.is_dir() || !required {
                continue;
            }
            let output = root.join(file.enclosed_name().expect("Invalid portable path"));
            fs::create_dir_all(output.parent().unwrap()).unwrap();
            std::io::copy(&mut file, &mut fs::File::create_new(output).unwrap()).unwrap();
        }
    } else {
        fs::copy(env!("CARGO_BIN_EXE_obz"), root.join("obz.exe")).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_obz-worker"), root.join("obz-worker.exe")).unwrap();
        let runtime = root.join("runtime/nanazip");
        fs::create_dir_all(&runtime).unwrap();
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/nanazip");
        for file in fs::read_dir(bundled).unwrap().map(Result::unwrap) {
            if file.file_type().unwrap().is_file() {
                fs::copy(file.path(), runtime.join(file.file_name())).unwrap();
            }
        }
    }
}

#[test]
fn cancel_selected_zip_verification_rolls_back_the_selected_output() {
    const CHILD: &str = "QING_TEST_SELECTED_ZIP_CANCEL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let package = tempfile::tempdir().unwrap();
        let host = package.path().join("test-host.exe");
        fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
        stage_cancellation_helpers(package.path());
        let result = Command::new(host)
            .args(["--exact", "cancel_selected_zip_verification_rolls_back_the_selected_output", "--nocapture"])
            .env(CHILD, "1")
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success() && stderr.contains("OA_PHASE direct-extract "), "{stderr}");
        let reads = stderr.lines()
            .find_map(|line| line.strip_prefix("OA_DIRECT_ZIP_READ_OPERATIONS "))
            .expect("missing native ZIP worker read counter")
            .parse::<u64>().unwrap();
        // Cancellation follows the 25,001st member. The same worker without
        // the buffer issues tens of thousands of small reads by that point.
        assert!((1..3_000).contains(&reads), "ZIP read-ahead not witnessed: {reads} reads");
        let worker = fs::read(package.path().join("obz-worker.exe")).unwrap();
        let hash: String = Sha256::digest(worker).iter().map(|byte| format!("{byte:02x}")).collect();
        println!("QING_SELECTED_ZIP_CANCEL_PASS worker={hash} reads={reads}");
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("ordinary.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&source).unwrap());
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for index in 0..50_000 {
        zip.start_file(format!("file-{index:05}.txt"), stored).unwrap();
        zip.write_all(b"ordinary selected extraction cancellation fixture").unwrap();
    }
    zip.finish().unwrap();
    let source_bytes = fs::read(&source).unwrap();
    let destination = directory.path().join("output");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("unrelated.txt"), b"preserve").unwrap();

    let control = obz_core::engine::JobControl::new();
    let options = obz_core::engine::EngineOptions {
        selected: vec!["file-25000.txt".into()],
        threads: Some(4),
        control: Some(control.clone()),
        ..Default::default()
    };
    let job_source = source.clone();
    let job_destination = destination.clone();
    let job = thread::spawn(move || {
        obz_core::engine::extract(
            job_source.to_str().unwrap(), job_destination.to_str().unwrap(), &options,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let progress = control.snapshot();
        if progress.phase == "decoding" && progress.work.is_some_and(|work| work.items_done == 1) {
            break;
        }
        assert!(!job.is_finished() && Instant::now() < deadline, "selected output was never committed during verification");
        thread::sleep(Duration::from_millis(1));
    }
    // Opening without writing witnesses the retained source lock during decode.
    assert!(fs::File::options().write(true).open(&source).is_err());
    control.cancel();
    assert!(job.join().unwrap().unwrap_err().contains("cancelled"));
    fs::File::options().write(true).open(&source).unwrap();
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(destination.join("unrelated.txt")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
}

#[test]
fn corrupt_unselected_payload_rolls_back_earlier_output_and_preserves_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("input.zip");
    archive(&source);
    let mut original = fs::read(&source).unwrap();
    let bad = b"distinct content we can corrupt";
    let start = original
        .windows(bad.len())
        .position(|window| window == bad)
        .unwrap();
    original[start] ^= 1;
    fs::write(&source, &original).unwrap();
    let destination = temp.path().join("existing");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("unrelated.txt"), b"preserve").unwrap();
    let result = extract(&source, &destination, &["--include", "folder/good.txt"]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        !result.status.success() && stderr.contains("archive CRC verification failed"),
        "{stderr}"
    );
    assert!(!destination.join("folder").exists());
    assert!(!destination.join("bad.txt").exists());
    assert_eq!(
        fs::read(destination.join("unrelated.txt")).unwrap(),
        b"preserve"
    );
    assert_eq!(fs::read(source).unwrap(), original);
}
