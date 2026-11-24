use obz_core::{engine::EngineOptions, operations::{execute, Request}};
use std::{fs, path::Path};

/// Stored entries whose names are UTF-8 without general-purpose bit 11, as
/// written by macOS Archive Utility.
fn unflagged_zip(path: &Path, names: &[&[u8]]) {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (index, name) in names.iter().enumerate() {
        let data = format!("data{index}").into_bytes();
        let crc = crc32(&data);
        let offset = out.len() as u32;
        let len = data.len() as u32;
        out.extend(b"PK\x03\x04");
        out.extend([20u16, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        out.extend([crc, len, len].iter().flat_map(|v| v.to_le_bytes()));
        out.extend([name.len() as u16, 0].iter().flat_map(|v| v.to_le_bytes()));
        out.extend(*name);
        out.extend(&data);
        central.extend(b"PK\x01\x02");
        central.extend([20u16, 20, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        central.extend([crc, len, len].iter().flat_map(|v| v.to_le_bytes()));
        central.extend([name.len() as u16, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        central.extend([32u32, offset].iter().flat_map(|v| v.to_le_bytes()));
        central.extend(*name);
    }
    let directory = out.len() as u32;
    let count = names.len() as u16;
    out.extend(&central);
    out.extend(b"PK\x05\x06");
    out.extend([0u16, 0, count, count].iter().flat_map(|v| v.to_le_bytes()));
    out.extend([central.len() as u32, directory].iter().flat_map(|v| v.to_le_bytes()));
    out.extend(0u16.to_le_bytes());
    fs::write(path, out).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn request(operation: &str, path: &Path, dest: &Path, codepage: Option<u32>) -> Request {
    Request {
        operation: operation.into(),
        path: path.to_string_lossy().into_owned(),
        dest: dest.to_string_lossy().into_owned(),
        options: EngineOptions { codepage, ..Default::default() },
        ..Default::default()
    }
}

#[test]
fn macos_zip_names_are_listed_and_extracted_as_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("mac.zip");
    unflagged_zip(&archive, &["报表/数据.txt".as_bytes(), b"a.txt"]);
    let listed = execute(request("list", &archive, dir.path(), None)).unwrap();
    let names: Vec<_> = listed["entries"].as_array().unwrap().iter()
        .map(|entry| entry["name"].as_str().unwrap().replace('\\', "/")).collect();
    assert!(names.contains(&"报表/数据.txt".to_string()), "{names:?}");
    let out = dir.path().join("out");
    execute(request("extract", &archive, &out, None)).unwrap();
    assert_eq!(fs::read(out.join("报表").join("数据.txt")).unwrap(), b"data0");
    let path = archive.to_str().unwrap();
    let options = EngineOptions { selected: vec!["报表/数据.txt".into()], ..Default::default() };
    let selected = dir.path().join("selected");
    obz_core::operations::extract_archive(path, selected.to_str().unwrap(), &options).unwrap();
    assert_eq!(fs::read(selected.join("报表").join("数据.txt")).unwrap(), b"data0");
    assert!(!selected.join("a.txt").exists());
    assert_eq!(obz_core::operations::preview_archive(path, "报表/数据.txt", &EngineOptions::default()).unwrap(), b"data0");
    // An explicit code page is still the user's choice.
    let legacy = dir.path().join("legacy");
    execute(request("extract", &archive, &legacy, Some(936))).unwrap();
    assert!(!legacy.join("报表").exists());
}

#[test]
fn legacy_gbk_names_keep_the_system_code_page() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("gbk.zip");
    unflagged_zip(&archive, &[b"\xb1\xa8\xb1\xed.txt"]); // 报表.txt in GBK
    let out = dir.path().join("out");
    execute(request("extract", &archive, &out, Some(936))).unwrap();
    assert_eq!(fs::read(out.join("报表.txt")).unwrap(), b"data0");
}

#[cfg(windows)]
#[test]
fn nested_utf8_member_can_be_saved_back() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("outer.zip");
    let member = "数据.txt";
    unflagged_zip(&archive, &[member.as_bytes()]);
    let options = EngineOptions::default();
    let path = archive.to_str().unwrap();
    let mut opened = obz_core::engine::nested::NestedArchive::open(path, member, &options).unwrap();
    fs::write(opened.path(), b"new data").unwrap();
    opened.save(&options).unwrap();
    assert_eq!(obz_core::operations::preview_archive(path, member, &options).unwrap(), b"new data");
}

#[test]
fn windows_invalid_member_names_are_mapped_like_7zip() {
    use std::io::Write;
    let cases = [("what?.txt", "what_.txt"), ("title:note.txt", "title_note.txt"), ("CON.txt", "_CON.txt"),
        ("name.", "name_"), ("dir. /x.txt", "dir__/x.txt"), ("fine.txt", "fine.txt")];
    for (mapped, expected) in cases {
        assert_eq!(obz_core::extract::member_path(mapped).unwrap(), Path::new(expected), "{mapped}");
    }
    for unsafe_name in ["../evil.txt", "/abs.txt", "C:/absolute.txt", "C:relative.txt", "a/../b", "a//b", "\\\\server\\x", "line\nPath = x"] {
        assert!(obz_core::extract::member_path(unsafe_name).is_err(), "{unsafe_name}");
    }
    // User-chosen names stay strict.
    assert!(obz_core::extract::relative_path("what?.txt").is_err());
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("odd.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (name, _) in cases {
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(name.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    let out = dir.path().join("out");
    execute(request("extract", &archive, &out, None)).unwrap();
    for (name, expected) in cases {
        assert_eq!(fs::read(out.join(expected)).unwrap(), name.as_bytes(), "{name}");
    }
    // Selecting a mapped member still extracts it.
    let selected = dir.path().join("selected");
    let mut one = request("extract", &archive, &selected, None);
    one.options.selected = vec!["what?.txt".into()];
    execute(one).unwrap();
    assert_eq!(fs::read(selected.join("what_.txt")).unwrap(), b"what?.txt");
}
