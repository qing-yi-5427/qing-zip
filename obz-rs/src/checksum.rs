//! Single-pass file checksums. This module never opens files for writing.
use crate::engine::{EngineOptions, JobControl};
use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use std::io::Read;

#[derive(Clone, serde::Serialize)]
pub struct Algorithm {
    pub name: String,
    pub bytes: usize,
}

/// Discover only concrete algorithms advertised by the active, bundled engine.
/// No caller-supplied value is used as an engine switch until it matches this list.
pub fn algorithms(opts: &EngineOptions) -> Result<Vec<Algorithm>, String> {
    let args = vec!["i".into(), "-sccUTF-8".into(), "-bsp0".into()];
    let bytes = crate::engine::run_hash(&args, opts)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "invalid engine algorithm listing")?;
    let section = text.split_once("Hashers:").ok_or("engine did not list hash algorithms")?.1;
    let mut found = Vec::<Algorithm>::new();
    for line in section.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 4 { continue; }
        let Ok(bytes) = fields[1].parse::<usize>() else { continue };
        let name = fields[3];
        if !(1..=128).contains(&bytes) || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || found.iter().any(|a| a.name.eq_ignore_ascii_case(name)) { continue; }
        found.push(Algorithm { name: name.to_string(), bytes });
    }
    if found.is_empty() || found.len() > 128 { return Err("invalid engine hash algorithm list".into()); }
    Ok(found)
}

/// Empty selection retains the original four-hash, single-pass API. An explicit
/// selection (or "*") computes all requested hashes in one read of the file.
pub fn checksum_selected(path: &str, names: &[String], opts: &EngineOptions) -> Result<serde_json::Value, String> {
    if names.is_empty() { return checksum(path, opts); }
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    let mut file = HashInput::open(path)?;
    if let Some(selection) = CommonSelection::from_names(names) {
        return hash_common(path, &mut file.source, &file.before, selection, &ctl);
    }
    let chosen = choose_algorithms(names, opts)?;
    ctl.set("checksum", Some(0), "Calculating file checksums");
    let mut args = vec!["h".into(), "-sccUTF-8".into(), "-bsp2".into(), "-spd".into()];
    for a in &chosen { args.push(format!("-scrc{}", a.name)); }
    args.push("--".into());
    args.push(file.absolute.clone());
    let output = crate::engine::run_hash(&args, opts)?;
    let text = std::str::from_utf8(&output).map_err(|_| "invalid engine checksum output")?;
    let hashes = parse_hashes(text, &chosen)?;
    file.verify()?;
    ctl.check()?;
    ctl.set("complete", Some(100), "File checksums ready");
    Ok(serde_json::json!({"path":path,"size":file.before.len(),"hashes":hashes}))
}

fn hash_common(path: &str, file: &mut std::fs::File, before: &std::fs::Metadata, selection: CommonSelection, ctl: &JobControl) -> Result<serde_json::Value, String> {
    ctl.set("checksum", Some(0), "Calculating file checksums");
    let (size, hashes) = hash_reader(file, before.len(), ctl, selection)?;
    verify_hash_input(file, before)?;
    if size != before.len() {
        return Err("file changed while calculating checksums".into());
    }
    ctl.check()?;
    ctl.set("complete", Some(100), "File checksums ready");
    Ok(serde_json::json!({"path":path, "size":size, "hashes":hashes}))
}

struct HashInput {
    absolute: String,
    source: crate::engine_stream::PinnedSource,
    before: std::fs::Metadata,
}
impl HashInput {
    fn open(path: &str) -> Result<Self, String> {
        if path.is_empty() || path.contains(['\r','\n','\0']) { return Err("invalid checksum file path".into()); }
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let source = crate::engine_stream::pin(&path)?;
        let before = source.metadata().map_err(|e| e.to_string())?;
        let absolute = std::fs::canonicalize(path).map_err(|e| e.to_string())?
            .into_os_string().into_string().map_err(|_| "invalid checksum file path")?;
        Ok(Self { absolute, source, before })
    }
    fn verify(&self) -> Result<(), String> {
        verify_hash_input(&self.source, &self.before)
    }
}

fn verify_hash_input(file: &std::fs::File, before: &std::fs::Metadata) -> Result<(), String> {
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("file changed while calculating checksums".into());
    }
    Ok(())
}

fn choose_algorithms(names: &[String], opts: &EngineOptions) -> Result<Vec<Algorithm>, String> {
    let available = algorithms(opts)?;
    Ok(if names.len() == 1 && names[0] == "*" {
        available
    } else {
        if names.len() > 128 { return Err("too many checksum algorithms".into()); }
        let mut chosen = Vec::<Algorithm>::new();
        for name in names {
            let found = available.iter().find(|a| a.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("unsupported checksum algorithm: {name}"))?;
            if !chosen.iter().any(|a| a.name == found.name) { chosen.push(found.clone()); }
        }
        chosen
    })
}

mod batch;
#[cfg(windows)]
mod pin_batch;
pub use batch::checksum_files;

fn parse_hashes(text: &str, chosen: &[Algorithm]) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut hashes = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some((name, digest)) = line.split_once(" for data:") else { continue };
        let name = name.trim();
        let Some(a) = chosen.iter().find(|a| a.name == name) else { continue };
        let digest = digest.trim();
        if digest.len() != a.bytes * 2 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || hashes.insert(a.name.clone(), digest.to_ascii_lowercase()).is_some() {
            return Err("invalid engine checksum result".into());
        }
    }
    if hashes.len() != chosen.len() { return Err("engine checksum result is incomplete".into()); }
    Ok(hashes)
}

pub fn checksum(path: &str, opts: &EngineOptions) -> Result<serde_json::Value, String> {
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        // Restoring the timestamp after a same-size write can defeat metadata checks.
        open.share_mode(1);
    }
    let mut file = open.open(path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("checksums require a regular file".into());
    }
    ctl.set("checksum", Some(0), "Calculating file checksums");
    let (size, hashes) = hash_reader(&mut file, before.len(), &ctl, CommonSelection::ALL)?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || size != before.len()
    {
        return Err("file changed while calculating checksums".into());
    }
    ctl.check()?;
    ctl.set("complete", Some(100), "File checksums ready");
    Ok(
        serde_json::json!({"path":path, "size":size, "md5":hashes["MD5"],
        "sha1":hashes["SHA1"], "sha256":hashes["SHA256"], "crc32":hashes["CRC32"]}),
    )
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}

#[derive(Clone, Copy, Default)]
struct CommonSelection {
    md5: bool,
    sha1: bool,
    sha256: bool,
    crc32: bool,
}
impl CommonSelection {
    const ALL: Self = Self { md5:true, sha1:true, sha256:true, crc32:true };

    fn from_names(names: &[String]) -> Option<Self> {
        if names.len() > 128 { return None; }
        let mut selection = Self::default();
        for name in names {
            match name.to_ascii_uppercase().as_str() {
                "MD5" => selection.md5 = true,
                "SHA1" => selection.sha1 = true,
                "SHA256" => selection.sha256 = true,
                "CRC32" => selection.crc32 = true,
                _ => return None,
            }
        }
        Some(selection)
    }
}

fn hash_reader(
    reader: &mut impl Read,
    total: u64,
    ctl: &JobControl,
    selected: CommonSelection,
) -> Result<(u64, std::collections::BTreeMap<&'static str, String>), String> {
    let mut md5 = selected.md5.then(Md5::new);
    let mut sha1 = selected.sha1.then(Sha1::new);
    let mut sha256 = selected.sha256.then(Sha256::new);
    let mut crc = selected.crc32.then(crc32fast::Hasher::new);
    let mut buffer = vec![0u8; total.clamp(1, 1024 * 1024) as usize];
    let mut size = 0u64;
    loop {
        ctl.check()?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("file size overflow")?;
        if let Some(h) = &mut md5 { h.update(&buffer[..n]); }
        if let Some(h) = &mut sha1 { h.update(&buffer[..n]); }
        if let Some(h) = &mut sha256 { h.update(&buffer[..n]); }
        if let Some(h) = &mut crc { h.update(&buffer[..n]); }
        let percent = if total == 0 {
            0
        } else {
            ((size as u128 * 100 / total as u128).min(99)) as u8
        };
        ctl.set("checksum", Some(percent), "Calculating file checksums");
    }
    ctl.check()?;
    let mut hashes = std::collections::BTreeMap::new();
    if let Some(h) = md5 { hashes.insert("MD5", hex(&h.finalize())); }
    if let Some(h) = sha1 { hashes.insert("SHA1", hex(&h.finalize())); }
    if let Some(h) = sha256 { hashes.insert("SHA256", hex(&h.finalize())); }
    if let Some(h) = crc { hashes.insert("CRC32", format!("{:08x}", h.finalize())); }
    Ok((size, hashes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_known_vectors_and_all_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checksum-中文.txt");
        std::fs::write(&path, b"abc").unwrap();
        let opts = EngineOptions::default();
        let result = checksum_selected(path.to_str().unwrap(), &["SHA3-256".into(), "SHA512".into(), "BLAKE3".into()], &opts).unwrap();
        assert_eq!(result["hashes"]["SHA3-256"], "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532");
        assert_eq!(result["hashes"]["SHA512"], "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
        assert_eq!(result["hashes"]["BLAKE3"], "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");
        let all = checksum_selected(path.to_str().unwrap(), &["*".into()], &opts).unwrap();
        assert_eq!(all["hashes"].as_object().unwrap().len(), algorithms(&opts).unwrap().len());
        assert_eq!(all["hashes"]["SHA256"], checksum(path.to_str().unwrap(), &opts).unwrap()["sha256"]);
        assert_eq!(all["hashes"]["CRC32"], "352441c2");
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        std::fs::write(&path, b"").unwrap();
        let empty = checksum_selected(path.to_str().unwrap(), &["SHA256".into()], &opts).unwrap();
        assert_eq!(empty["hashes"]["SHA256"], "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
    #[test]
    fn extended_rejects_unknown_switches_directories_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.txt");
        std::fs::write(&path, b"abc").unwrap();
        for name in ["-oanything", "SHA256 -psecret", "*\n"] {
            assert!(checksum_selected(path.to_str().unwrap(), &[name.into()], &EngineOptions::default()).is_err());
        }
        assert!(checksum_selected(dir.path().to_str().unwrap(), &["SHA256".into()], &EngineOptions::default()).is_err());
        let ctl = JobControl::new(); ctl.cancel();
        let opts = EngineOptions { control: Some(ctl), ..Default::default() };
        assert!(checksum_selected(path.to_str().unwrap(), &["SHA256".into()], &opts).unwrap_err().contains("cancelled"));
    }
    #[test]
    fn checksum_parser_rejects_missing_duplicate_and_malformed_values() {
        let chosen = [Algorithm { name: "CRC32".into(), bytes: 4 }];
        assert!(parse_hashes("CRC32 for data: 352441c2", &chosen).is_ok());
        for text in ["", "CRC32 for data: 352441c", "CRC32 for data: Z52441c2", "CRC32 for data: 352441c2\nCRC32 for data: 352441c2"] {
            assert!(parse_hashes(text, &chosen).is_err());
        }
    }
    #[test]
    fn published_vectors_and_empty_file() {
        let ctl = JobControl::new();
        let h = hash_reader(&mut &b"abc"[..], 3, &ctl, CommonSelection::ALL).unwrap();
        assert_eq!(h.1["MD5"], "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.1["SHA1"], "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h.1["SHA256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(h.1["CRC32"], "352441c2");
        let h = hash_reader(&mut &b""[..], 0, &ctl, CommonSelection::ALL).unwrap();
        assert_eq!(h.1["MD5"], "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(h.1["SHA1"], "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            h.1["SHA256"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(h.1["CRC32"], "00000000");
    }
    #[test]
    fn multiple_buffers_match_independent_hashlib_vectors() {
        let data = vec![b'a'; 2_000_000];
        let h = hash_reader(&mut data.as_slice(), data.len() as u64, &JobControl::new(), CommonSelection::ALL).unwrap();
        assert_eq!(h.0, 2_000_000);
        assert_eq!(h.1["MD5"], "2a915e52d86d42e58e580f4073120a6b");
        assert_eq!(h.1["SHA1"], "46aa62723f78ff6e2e381d21988a801db99c2a32");
        assert_eq!(
            h.1["SHA256"],
            "bcf7f9d1b4311c3352e60502255ce09a6744df84e8f2c89f79c4b5d74933a95a"
        );
        assert_eq!(h.1["CRC32"], "eab6f954");
    }
    #[test]
    fn file_is_unchanged_and_cancelled_request_fails() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("中文.txt");
        std::fs::write(&path, b"abc").unwrap();
        let result = checksum(path.to_str().unwrap(), &EngineOptions::default()).unwrap();
        assert_eq!(result["size"], 3);
        assert_eq!(result["crc32"], "352441c2");
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        let ctl = JobControl::new();
        ctl.cancel();
        assert!(checksum(
            path.to_str().unwrap(),
            &EngineOptions {
                control: Some(ctl),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled"));
        assert!(checksum(t.path().to_str().unwrap(), &EngineOptions::default()).is_err());
    }
    #[test]
    fn cancellation_during_streaming_stops_before_next_read() {
        struct CancellingReader {
            ctl: JobControl,
            reads: usize,
        }
        impl Read for CancellingReader {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                b[0] = 42;
                self.ctl.cancel();
                Ok(1)
            }
        }
        let ctl = JobControl::new();
        let mut reader = CancellingReader {
            ctl: ctl.clone(),
            reads: 0,
        };
        assert!(hash_reader(&mut reader, 100, &ctl, CommonSelection::ALL)
            .unwrap_err()
            .contains("cancelled"));
        assert_eq!(reader.reads, 1);
    }
}
