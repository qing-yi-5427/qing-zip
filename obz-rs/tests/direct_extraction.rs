#![cfg(all(windows, target_pointer_width = "64", target_env = "msvc"))]

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output},
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
