// Real NanaZip archives cover both password callbacks. The selected worker must
// verify an unrelated damaged block before publishing a correct selected file.
#![cfg(nana_selected_worker)]
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
const PASSWORD: &str = "fixture-only-\u{9752}\u{1F5C4}";

fn run(cmd: &mut Command, password: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{password}").unwrap();
    let out = child.wait_with_output().unwrap();
    let hidden = |bytes: &[u8]| !String::from_utf8_lossy(bytes).contains(password);
    assert!(hidden(&out.stdout) && hidden(&out.stderr));
    out
}

#[test]
fn encrypted_split_selection_checks_unselected_blocks_before_publication() {
    verify_encrypted_split_method("LZMA2");
}

#[test]
fn encrypted_lzma_split_checks_unselected_blocks_before_publication() {
    verify_encrypted_split_method("LZMA");
}

fn verify_encrypted_split_method(method: &str) {
    let nana = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join(obz_core::engine::BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let scratch = root.join("scratch");
    fs::create_dir(&scratch).unwrap();
    let inputs = root.join("inputs");
    fs::create_dir(&inputs).unwrap();
    let mut state = 0xfedc_ba98_u32;
    let mut names = Vec::new();
    for i in 0..6 {
        let name = format!("{i}.bin");
        let bytes: Vec<u8> = (0..32_768)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        fs::write(inputs.join(&name), bytes).unwrap();
        names.push(name);
    }
    for header in ["on", "off"] {
        let work = root.join(header);
        fs::create_dir(&work).unwrap();
        let archive = work.join("archive.7z");
        let first = work.join("archive.7z.001");
        let create = run(
            Command::new(&nana)
                .current_dir(&inputs)
                .args([
                    "a",
                    "-t7z",
                    "-ms=2f",
                    "-v8k",
                    "-sccUTF-8",
                    "-p",
                ])
                .arg(format!("-m0={method}"))
                .arg(format!("-mhe={header}"))
                .arg(&archive)
                .args(&names),
            PASSWORD,
        );
        assert!(create.status.success());
        let extract = |dest: &Path, password: &str| {
            run(
                Command::new(env!("CARGO_BIN_EXE_obz"))
                    .arg("extract")
                    .arg(&first)
                    .arg(dest)
                    .args(["--include", "4.bin", "--password-stdin"])
                    .env("TEMP", &scratch)
                    .env("TMP", &scratch)
                    .env("OPEN_ARCHIVE_PROFILE", "1"),
                password,
            )
        };
        let valid = work.join("valid");
        let out = extract(&valid, PASSWORD);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stderr).contains("OA_SELECTED_WORKER status=ok"));
        assert_eq!(fs::read_dir(&valid).unwrap().count(), 1);
        assert_eq!(
            fs::read(valid.join("4.bin")).unwrap(),
            fs::read(inputs.join("4.bin")).unwrap()
        );
        let failed = work.join("failed");
        assert!(!extract(&failed, "fixture-wrong").status.success());
        assert!(!failed.exists());
        let mut data = fs::read(&first).unwrap();
        data[2048] ^= 1;
        fs::write(&first, data).unwrap();
        let damaged = extract(&failed, PASSWORD);
        assert!(!damaged.status.success());
        assert!(
            String::from_utf8_lossy(&damaged.stderr).contains("OA_SELECTED_WORKER status=error")
        );
        assert!(!failed.exists());
        assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    }
}
