//! Single-pass file checksums. This module never opens files for writing.
use crate::engine::{EngineOptions, JobControl};
use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use std::{fs::File, io::Read};

pub fn checksum(path: &str, opts: &EngineOptions) -> Result<serde_json::Value, String> {
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("checksums require a regular file".into());
    }
    ctl.set("checksum", Some(0), "Calculating file checksums");
    let result = hash_reader(&mut file, before.len(), &ctl)?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || result.0 != before.len()
    {
        return Err("file changed while calculating checksums".into());
    }
    ctl.check()?;
    ctl.set("complete", Some(100), "File checksums ready");
    Ok(
        serde_json::json!({"path":path, "size":result.0, "md5":result.1,
        "sha1":result.2, "sha256":result.3, "crc32":result.4}),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_reader(
    reader: &mut impl Read,
    total: u64,
    ctl: &JobControl,
) -> Result<(u64, String, String, String, String), String> {
    let (mut md5, mut sha1, mut sha256, mut crc) = (
        Md5::new(),
        Sha1::new(),
        Sha256::new(),
        crc32fast::Hasher::new(),
    );
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        ctl.check()?;
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("file size overflow")?;
        md5.update(&buffer[..n]);
        sha1.update(&buffer[..n]);
        sha256.update(&buffer[..n]);
        crc.update(&buffer[..n]);
        let percent = if total == 0 {
            0
        } else {
            ((size as u128 * 100 / total as u128).min(99)) as u8
        };
        ctl.set("checksum", Some(percent), "Calculating file checksums");
    }
    ctl.check()?;
    Ok((
        size,
        hex(&md5.finalize()),
        hex(&sha1.finalize()),
        hex(&sha256.finalize()),
        format!("{:08x}", crc.finalize()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_vectors_and_empty_file() {
        let ctl = JobControl::new();
        let h = hash_reader(&mut &b"abc"[..], 3, &ctl).unwrap();
        assert_eq!(h.1, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.2, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h.3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(h.4, "352441c2");
        let h = hash_reader(&mut &b""[..], 0, &ctl).unwrap();
        assert_eq!(h.1, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(h.2, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            h.3,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(h.4, "00000000");
    }
    #[test]
    fn multiple_buffers_match_independent_hashlib_vectors() {
        let data = vec![b'a'; 2_000_000];
        let h = hash_reader(&mut data.as_slice(), data.len() as u64, &JobControl::new()).unwrap();
        assert_eq!(h.0, 2_000_000);
        assert_eq!(h.1, "2a915e52d86d42e58e580f4073120a6b");
        assert_eq!(h.2, "46aa62723f78ff6e2e381d21988a801db99c2a32");
        assert_eq!(
            h.3,
            "bcf7f9d1b4311c3352e60502255ce09a6744df84e8f2c89f79c4b5d74933a95a"
        );
        assert_eq!(h.4, "eab6f954");
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
        assert!(hash_reader(&mut reader, 100, &ctl)
            .unwrap_err()
            .contains("cancelled"));
        assert_eq!(reader.reads, 1);
    }
}
