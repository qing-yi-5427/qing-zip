//! Full integrity verification of ordinary ZIP proposals, with no output files.
//! Unsupported layouts retain the existing engine test and its format policy.
use super::*;

pub(super) fn ordinary_zip(path: &Path, opts: &EngineOptions) -> Result<bool, String> {
    let policy = crate::machine_policy::scope();
    if policy.restricted() || opts.password.is_some() || opts.codepage.is_some() {
        return Ok(false);
    }
    let ctl = control(opts);
    ctl.check()?;
    let mut source = crate::engine_stream::pin(path)?;
    // Bound this optimization to small proposals. Larger files retain the
    // native verifier without an additional header-by-header disk pass.
    let length = source.metadata().map_err(|e| e.to_string())?.len();
    if length > 8 * 1024 * 1024 { return Ok(false); }
    let mut bytes = Vec::with_capacity(length as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        ctl.check()?;
        let n = source.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        if bytes.len() + n > length as usize { return Err("ZIP proposal changed".into()); }
        bytes.extend_from_slice(&chunk[..n]);
    }
    if bytes.len() != length as usize { return Err("ZIP proposal changed".into()); }
    if bytes.len() < 22 { return Ok(false); }
    // Enforce layout/count bounds before ZipArchive allocates member metadata.
    let Some(end) = (bytes.len().saturating_sub(65557)..=bytes.len() - 22).rev().find(|&i| {
        bytes[i..i + 4] == *b"PK\x05\x06"
            && i + 22 + zip_u16(&bytes, i + 20) as usize == bytes.len()
    }) else { return Ok(false); };
    let count = zip_u16(&bytes, end + 10) as usize;
    if !ordinary_zip_layout_count(&bytes, count) {
        return Ok(false);
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
    if archive.len() != count || archive.offset() != 0 || archive.has_duplicate_names()
        || archive.validate_header_consistency().is_err() {
        return Ok(false);
    }
    // Qualify every member before using this decoder. Do not reinterpret
    // encrypted, linked, special, legacy-name, or directory-payload members.
    let mut decoded_total = 0u64;
    for index in 0..count {
        ctl.check()?;
        let member = archive.by_index_data(index).map_err(|e| e.to_string())?;
        let kind = member.unix_mode().map(|m| m & 0xf000).unwrap_or(0);
        decoded_total = match decoded_total.checked_add(member.size()) {
            Some(total) if total <= 64 * 1024 * 1024 => total,
            _ => return Ok(false),
        };
        if member.encrypted() || member.is_symlink()
            || !matches!(kind, 0 | 0x4000 | 0x8000)
            || member.external_attributes() & 0x400 != 0
            || std::str::from_utf8(member.name_raw()).is_err()
            || (member.is_dir() && member.size() != 0)
        {
            return Ok(false);
        }
    }
    let _timer = PhaseTimer::new("verify-edited-zip");
    ctl.set("testing", Some(0), "Checking archive integrity");
    let mut buffer = vec![0u8; 256 * 1024];
    for index in 0..count {
        ctl.check()?;
        let mut member = archive.by_index(index).map_err(|e| format!("ZIP integrity: {e}"))?;
        let expected_size = member.size();
        let expected_crc = member.crc32();
        let mut size = 0u64;
        let mut crc = crc32fast::Hasher::new();
        loop {
            ctl.check()?;
            let n = member.read(&mut buffer).map_err(|e| format!("ZIP integrity: {e}"))?;
            if n == 0 { break; }
            size = size.checked_add(n as u64).ok_or("ZIP decoded size overflow")?;
            if size > expected_size { return Err("ZIP decoded length mismatch".into()); }
            crc.update(&buffer[..n]);
        }
        if size != expected_size || crc.finalize() != expected_crc {
            return Err("ZIP integrity mismatch".into());
        }
    }
    ctl.check()?;
    policy.validate_output()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(method: zip::CompressionMethod) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("edit.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        writer.start_file("first.txt", options).unwrap();
        writer.write_all(b"selected content").unwrap();
        writer.start_file("last.txt", options).unwrap();
        writer.write_all(&vec![42; 1024 * 1024]).unwrap();
        writer.add_directory("empty/", options).unwrap();
        writer.finish().unwrap();
        (temp, path)
    }

    #[test]
    fn edited_zip_verifies_stored_deflated_and_empty_archives_without_writing() {
        for method in [zip::CompressionMethod::Stored, zip::CompressionMethod::Deflated] {
            let (temp, path) = fixture(method);
            let before = fs::read(&path).unwrap();
            assert!(ordinary_zip(&path, &EngineOptions::default()).unwrap());
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("empty.zip");
        zip::ZipWriter::new(File::create(&path).unwrap()).finish().unwrap();
        assert!(ordinary_zip(&path, &EngineOptions::default()).unwrap());
    }

    #[test]
    fn edited_zip_rejects_corruption_even_outside_selection() {
        let (_temp, path) = fixture(zip::CompressionMethod::Stored);
        let offset = {
            let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            let offset = archive.by_name("last.txt").unwrap().data_start().unwrap();
            offset
        };
        let mut bytes = fs::read(&path).unwrap();
        bytes[offset as usize] ^= 1;
        fs::write(&path, &bytes).unwrap();
        let opts = EngineOptions { selected: vec!["first.txt".into()], ..EngineOptions::default() };
        assert!(ordinary_zip(&path, &opts).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn edited_zip_keeps_unsupported_layouts_on_existing_verifier() {
        let (_temp, path) = fixture(zip::CompressionMethod::Stored);
        for opts in [
            EngineOptions { password: Some("password".into()), ..EngineOptions::default() },
            EngineOptions { codepage: Some(936), ..EngineOptions::default() },
        ] {
            assert!(!ordinary_zip(&path, &opts).unwrap());
        }
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"trailing bytes");
        fs::write(&path, bytes).unwrap();
        assert!(!ordinary_zip(&path, &EngineOptions::default()).unwrap());
    }

    #[test]
    fn edited_zip_many_empty_members_keep_native_verification() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("many.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for i in 0..=MEMORY_ZIP_ENTRY_LIMIT {
            writer.start_file(format!("item-{i}.txt"), zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)).unwrap();
        }
        writer.finish().unwrap();
        assert!(path.metadata().unwrap().len() < 8 * 1024 * 1024);
        assert!(!ordinary_zip(&path, &EngineOptions::default()).unwrap());
    }

    #[test]
    fn edited_zip_high_expansion_keeps_native_verification() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("high-expansion.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer.start_file("data.bin", zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)).unwrap();
        let block = vec![42; 1024 * 1024];
        for _ in 0..65 { writer.write_all(&block).unwrap(); }
        writer.finish().unwrap();
        assert!(path.metadata().unwrap().len() < 8 * 1024 * 1024);
        assert!(!ordinary_zip(&path, &EngineOptions::default()).unwrap());
    }

    #[test]
    fn edited_zip_large_proposals_keep_native_verification() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        writer.start_file("large.bin", zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)).unwrap();
        writer.write_all(&vec![17; 8 * 1024 * 1024]).unwrap();
        writer.finish().unwrap();
        assert!(!ordinary_zip(&path, &EngineOptions::default()).unwrap());
    }

    #[test]
    fn edited_zip_checks_cancel_before_verifying() {
        let (_temp, path) = fixture(zip::CompressionMethod::Stored);
        let ctl = JobControl::new();
        ctl.cancel();
        let opts = EngineOptions { control: Some(ctl), ..EngineOptions::default() };
        assert!(ordinary_zip(&path, &opts).unwrap_err().contains("cancel"));
    }
}
