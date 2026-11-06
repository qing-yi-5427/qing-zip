use std::{fs, io::Write, process::Command};

#[test]
fn cli_thread_option_works_for_test_and_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file("payload.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"thread-limited decode").unwrap();
    zip.finish().unwrap();
    for threads in ["1", "3"] {
        let tested = Command::new(env!("CARGO_BIN_EXE_obz"))
            .args(["test", path.to_str().unwrap(), "--threads", threads])
            .output()
            .unwrap();
        assert!(
            tested.status.success(),
            "{}",
            String::from_utf8_lossy(&tested.stderr)
        );
        let output = temp.path().join(threads);
        let extracted = Command::new(env!("CARGO_BIN_EXE_obz"))
            .args([
                "extract",
                path.to_str().unwrap(),
                output.to_str().unwrap(),
                "--threads",
                threads,
            ])
            .output()
            .unwrap();
        assert!(
            extracted.status.success(),
            "{}",
            String::from_utf8_lossy(&extracted.stderr)
        );
        assert_eq!(
            fs::read(output.join("payload.txt")).unwrap(),
            b"thread-limited decode"
        );
    }
}
