use obz_core::{extract, setup, sevenz, zipfmt};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "obz-test-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
    fn zip(&self) -> Vec<u8> {
        fs::write(self.0.join("file.txt"), b"sample").unwrap();
        zipfmt::create_zip(&self.path("a.zip"), &[self.path("file.txt")], true).unwrap();
        fs::read(self.0.join("a.zip")).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn entries(data: &[u8]) -> Vec<zipfmt::ZipEntry> {
    zipfmt::read_central_dir(data, 0, data.len()).unwrap()
}

#[test]
fn unsafe_names_are_rejected() {
    for name in [
        "../x",
        "a/../../x",
        "..\\x",
        "/x",
        "C:/x",
        "C:x",
        "//server/x",
        "a:x",
        "a./x",
        "NUL",
        "CON.txt",
        "a/../",
        "a\0b",
    ] {
        assert!(extract::relative_path(name).is_err(), "{name}");
    }
    assert!(extract::relative_path("中文/hello.txt").is_ok());
}

#[test]
fn zip_traversal_is_rejected_before_any_write() {
    let t = Temp::new();
    let data = t.zip();
    let mut list = entries(&data);
    list[0].name = "../outside.txt".into();
    assert!(extract::zip(&data, 0, &list, &t.path("out")).is_err());
    assert!(!t.0.join("outside.txt").exists());
    assert!(!t.0.join("out").exists());
}

#[test]
fn sevenz_traversal_is_rejected() {
    let t = Temp::new();
    let data = include_bytes!("fixtures/emptyfile.7z");
    let mut archive = sevenz::parse_archive(data).unwrap();
    archive.entries[0].name = "../escape".into();
    assert!(extract::sevenz(&archive, data, &t.path("out")).is_err());
    assert!(!t.0.join("escape").exists());
}

#[test]
fn truncated_zip_inputs_do_not_panic() {
    let t = Temp::new();
    let data = t.zip();
    for size in 0..data.len() {
        assert!(zipfmt::read_central_dir(&data[..size], 0, size).is_err());
    }
    assert!(zipfmt::read_central_dir(&data, 0, usize::MAX).is_err());
    assert!(zipfmt::read_central_dir(&data, data.len(), 0).is_err());
}

#[test]
fn invalid_zip_entry_offsets_and_sizes_return_errors() {
    let t = Temp::new();
    let data = t.zip();
    let mut list = entries(&data);
    list[0].local_offset = u32::MAX;
    assert!(zipfmt::extract_entry(&data, 0, &list[0]).is_err());
    list[0].local_offset = 0;
    list[0].comp_size = u64::MAX;
    assert!(zipfmt::extract_entry(&data, 0, &list[0]).is_err());
    list[0].comp_size = 6;
    list[0].size = 7;
    assert!(zipfmt::extract_entry(&data, 0, &list[0]).is_err());
}

#[test]
fn crc_error_never_overwrites_existing_file() {
    let t = Temp::new();
    let data = t.zip();
    let mut list = entries(&data);
    fs::create_dir(t.0.join("out")).unwrap();
    fs::write(t.0.join("out/file.txt"), b"keep").unwrap();
    list[0].crc ^= 1;
    assert!(extract::zip(&data, 0, &list, &t.path("out")).is_err());
    assert_eq!(fs::read(t.0.join("out/file.txt")).unwrap(), b"keep");
}

#[test]
fn sevenz_file_crc_is_checked_by_shared_gui_path() {
    let t = Temp::new();
    let data = include_bytes!("fixtures/bcj.7z");
    let mut archive = sevenz::parse_archive(data).unwrap();
    archive.entries[0].crc = Some(archive.entries[0].crc.unwrap() ^ 1);
    assert!(extract::sevenz(&archive, data, &t.path("out"))
        .unwrap_err()
        .contains("CRC"));
    assert!(!t.0.join("out/bcj.bin").exists());
}

#[test]
fn sevenz_folder_crc_is_checked() {
    let data = include_bytes!("fixtures/bcj.7z");
    let mut a = sevenz::parse_archive(data).unwrap();
    a.folders[0].unpack_crc = Some(0);
    assert!(a.read_folder(data, 0).unwrap_err().contains("CRC"));
}

#[test]
fn bcj_matches_external_7zip_fixture() {
    let t = Temp::new();
    let data = include_bytes!("fixtures/bcj.7z");
    let a = sevenz::parse_archive(data).unwrap();
    assert_eq!(extract::sevenz(&a, data, &t.path("out")).unwrap(), 1);
    assert_eq!(
        fs::read(t.0.join("out/bcj.bin")).unwrap(),
        include_bytes!("fixtures/bcj.bin")
    );
}

#[test]
fn sevenz_empty_file_is_created() {
    let t = Temp::new();
    let data = include_bytes!("fixtures/emptyfile.7z");
    assert_eq!(
        extract::sevenz(&sevenz::parse_archive(data).unwrap(), data, &t.path("out")).unwrap(),
        1
    );
    assert_eq!(fs::read(t.0.join("out/empty.txt")).unwrap(), b"");
}

#[test]
fn zip_empty_directory_is_created() {
    let t = Temp::new();
    let data = t.zip();
    let mut list = entries(&data);
    list[0].name = "empty/nested/".into();
    assert_eq!(extract::zip(&data, 0, &list, &t.path("out")).unwrap(), 0);
    assert!(t.0.join("out/empty/nested").is_dir());
}

#[test]
fn utf8_names_are_flagged_in_both_headers() {
    let t = Temp::new();
    fs::write(t.0.join("中文.txt"), b"hello").unwrap();
    zipfmt::create_zip(&t.path("utf8.zip"), &[t.path("中文.txt")], false).unwrap();
    let data = fs::read(t.0.join("utf8.zip")).unwrap();
    let cd = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    for pos in [6, cd + 8] {
        assert_ne!(u16::from_le_bytes([data[pos], data[pos + 1]]) & 0x800, 0);
    }
    assert_eq!(entries(&data)[0].name, "中文.txt");
}

#[test]
fn overwrite_does_not_modify_other_hard_links() {
    let t = Temp::new();
    let data = t.zip();
    fs::create_dir(t.0.join("out")).unwrap();
    fs::write(t.0.join("outside"), b"keep").unwrap();
    fs::hard_link(t.0.join("outside"), t.0.join("out/file.txt")).unwrap();
    extract::zip(&data, 0, &entries(&data), &t.path("out")).unwrap();
    assert_eq!(fs::read(t.0.join("outside")).unwrap(), b"keep");
    assert_eq!(fs::read(t.0.join("out/file.txt")).unwrap(), b"sample");
}

#[test]
fn empty_zip_is_readable() {
    let t = Temp::new();
    zipfmt::create_zip(&t.path("empty.zip"), &[], false).unwrap();
    assert!(entries(&fs::read(t.0.join("empty.zip")).unwrap()).is_empty());
}

#[cfg(windows)]
#[test]
fn directory_junction_cannot_escape_destination() {
    let t = Temp::new();
    let data = t.zip();
    let mut list = entries(&data);
    fs::create_dir(t.0.join("out")).unwrap();
    fs::create_dir(t.0.join("outside")).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args([
            "/C",
            "mklink",
            "/J",
            &t.path("out/link").replace('/', "\\"),
            &t.path("outside"),
        ])
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stdout));
    list[0].name = "link/file.txt".into();
    assert!(extract::zip(&data, 0, &list, &t.path("out")).is_err());
    assert!(!t.0.join("outside/file.txt").exists());
    fs::remove_dir(t.0.join("out/link")).unwrap();
}

#[test]
fn damaged_zip_fields_do_not_panic() {
    let t = Temp::new();
    let data = t.zip();
    for offset in 0..data.len() {
        let mut damaged = data.clone();
        damaged[offset] ^= 0xff;
        if let Ok(list) = zipfmt::read_central_dir(&damaged, 0, damaged.len()) {
            for e in list {
                let _ = zipfmt::extract_entry(&damaged, 0, &e);
            }
        }
    }
}

#[test]
fn installer_payload_and_ui_share_path_policy() {
    let t = Temp::new();
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../BANDIZIP-SETUP-BETA.exe");
    let data = fs::read(sample).unwrap();
    let mut pkg = setup::open(&data).unwrap();
    pkg.archive.entries[0].name = "../escape".into();
    assert!(setup::extract_payload(&pkg, &t.path("payload")).is_err());
    pkg.ui_entries[0].name = "../escape".into();
    assert!(setup::extract_ui(&pkg, &t.path("ui")).is_err());
    assert!(!t.0.join("escape").exists());
}
