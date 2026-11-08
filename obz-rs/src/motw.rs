//! Propagate the source archive's Internet-zone metadata before publishing files.
//! Windows backup APIs address streams through an already-open file handle: no
//! output path is reopened, and skipped/existing destination files are untouched.
use std::{fs::File, path::Path, sync::Arc};

const MAX_ZONE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct ZoneIdentifier {
    bytes: Vec<u8>,
    // Keeps the checked source identity and its streams stable until publication.
    // Private and serde-skipped through EngineOptions; never supplied by JSON.
    _source: Option<Arc<File>>,
}

impl ZoneIdentifier {
    // A private, non-serializable negative lookup cache. A caller-controlled
    // JSON request cannot use it to suppress archive-origin inspection.
    pub(crate) fn unmarked() -> Self {
        Self {
            bytes: Vec::new(),
            _source: None,
        }
    }
    pub(crate) fn is_marked(&self) -> bool {
        !self.bytes.is_empty()
    }

    fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_ZONE_BYTES {
            return Err("Internet zone metadata exceeds 64 KiB".into());
        }
        let text = if let Some(raw) = bytes.strip_prefix(&[0xff, 0xfe]) {
            if raw.len() % 2 != 0 {
                return Err("invalid Internet zone metadata".into());
            }
            String::from_utf16(
                &raw.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| "invalid Internet zone metadata")?
        } else {
            // Zone keys are ASCII; legacy Windows writers may use the system
            // codepage for URL values. Preserve the original stream unchanged.
            String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .into_owned()
        };
        if text.contains('\0') {
            return Err("invalid Internet zone metadata".into());
        }
        let mut in_zone = false;
        let mut zone = None;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_zone = line.eq_ignore_ascii_case("[ZoneTransfer]");
            }
            if !in_zone {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("ZoneId") {
                    let value = value.trim();
                    if !matches!(value, "0" | "1" | "2" | "3" | "4") || zone.is_some() {
                        return Err("invalid Internet zone metadata".into());
                    }
                    zone = Some(value);
                }
            }
        }
        if zone.is_none() {
            return Err("Internet zone metadata has no valid ZoneId".into());
        }
        Ok(Self {
            bytes,
            _source: None,
        })
    }
}

/// Read only the named stream, skipping archive data without decoding it.
pub fn from_archive(path: &Path) -> Result<Option<ZoneIdentifier>, String> {
    let context = archive_context(path)?;
    Ok(context.is_marked().then_some(context))
}

/// Capture marked or unmarked provenance and keep the checked source pinned
/// through extraction. A clone shares the same handle, without another lookup.
pub fn archive_context(path: &Path) -> Result<ZoneIdentifier, String> {
    #[cfg(windows)]
    {
        windows::archive_context(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(ZoneIdentifier::unmarked())
    }
}

/// The caller must pass its newly created temporary/staging file before it is
/// made visible as an extracted file. Requires write access, not backup privilege.
/// Filesystems without named streams cannot retain MOTW; do not fail on those.
pub fn apply_to_file(file: &File, zone: Option<&ZoneIdentifier>) -> Result<(), String> {
    let Some(zone) = zone else { return Ok(()) };
    if zone.bytes.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        windows::write_zone(file, &zone.bytes)
    }
    #[cfg(not(windows))]
    {
        let _ = (file, zone);
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn output_marker(file: &File) -> Result<Option<Vec<u8>>, String> {
    windows::read_zone(file).map(|value| value.map(|marker| marker.bytes))
}

#[cfg(windows)]
pub(crate) use windows::named_streams;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::c_void,
        fs::OpenOptions,
        io,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
    };
    type Handle = *mut c_void;
    #[link(name = "Kernel32")]
    extern "system" {
        fn BackupRead(
            file: Handle,
            buffer: *mut u8,
            size: u32,
            read: *mut u32,
            abort: i32,
            security: i32,
            context: *mut Handle,
        ) -> i32;
        fn BackupWrite(
            file: Handle,
            buffer: *const u8,
            size: u32,
            written: *mut u32,
            abort: i32,
            security: i32,
            context: *mut Handle,
        ) -> i32;
        fn BackupSeek(
            file: Handle,
            low: u32,
            high: u32,
            low_done: *mut u32,
            high_done: *mut u32,
            context: *mut Handle,
        ) -> i32;
        fn GetVolumeInformationByHandleW(
            file: Handle,
            volume: *mut u16,
            volume_len: u32,
            serial: *mut u32,
            component: *mut u32,
            flags: *mut u32,
            fs: *mut u16,
            fs_len: u32,
        ) -> i32;
    }
    struct Context<'a> {
        file: &'a File,
        raw: Handle,
        writing: bool,
    }
    impl Drop for Context<'_> {
        fn drop(&mut self) {
            if !self.raw.is_null() {
                let mut count = 0;
                unsafe {
                    if self.writing {
                        BackupWrite(
                            self.file.as_raw_handle(),
                            std::ptr::null(),
                            0,
                            &mut count,
                            1,
                            0,
                            &mut self.raw,
                        );
                    } else {
                        BackupRead(
                            self.file.as_raw_handle(),
                            std::ptr::null_mut(),
                            0,
                            &mut count,
                            1,
                            0,
                            &mut self.raw,
                        );
                    }
                }
            }
        }
    }
    fn error(action: &str) -> String {
        format!(
            "{action} Internet zone metadata: {}",
            io::Error::last_os_error()
        )
    }
    pub(crate) fn named_streams(file: &File) -> Result<bool, String> {
        let mut flags = 0;
        if unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(error("query filesystem for"));
        }
        Ok(flags & 0x0004_0000 != 0)
    }
    impl Context<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> Result<usize, String> {
            let mut n = 0;
            if unsafe {
                BackupRead(
                    self.file.as_raw_handle(),
                    bytes.as_mut_ptr(),
                    bytes.len() as u32,
                    &mut n,
                    0,
                    0,
                    &mut self.raw,
                )
            } == 0
            {
                return Err(error("read"));
            }
            Ok(n as usize)
        }
        fn exact(&mut self, bytes: &mut [u8]) -> Result<(), String> {
            let mut offset = 0;
            while offset < bytes.len() {
                let n = self.read(&mut bytes[offset..])?;
                if n == 0 {
                    return Err("truncated Internet zone stream".into());
                }
                offset += n;
            }
            Ok(())
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            let mut offset = 0;
            while offset < bytes.len() {
                let mut n = 0;
                if unsafe {
                    BackupWrite(
                        self.file.as_raw_handle(),
                        bytes[offset..].as_ptr(),
                        (bytes.len() - offset) as u32,
                        &mut n,
                        0,
                        0,
                        &mut self.raw,
                    )
                } == 0
                {
                    return Err(error("write"));
                }
                if n == 0 {
                    return Err("could not write Internet zone metadata".into());
                }
                offset += n as usize;
            }
            Ok(())
        }
    }
    pub(super) fn archive_context(path: &Path) -> Result<ZoneIdentifier, String> {
        crate::engine::no_links(path)?;
        // Keep the archive stable while reading its streams; do not follow a
        // reparse point substituted for the final component.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(path)
            .map_err(|e| format!("open archive zone metadata: {e}"))?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            return Err("refusing linked archive zone metadata".into());
        }
        let mut zone = read_zone(&file)?.unwrap_or_else(ZoneIdentifier::unmarked);
        zone._source = Some(Arc::new(file));
        Ok(zone)
    }
    pub(super) fn read_zone(file: &File) -> Result<Option<ZoneIdentifier>, String> {
        if !named_streams(&file)? {
            return Ok(None);
        }
        let mut context = Context {
            file: &file,
            raw: std::ptr::null_mut(),
            writing: false,
        };
        for _ in 0..1024 {
            let mut header = [0u8; 20];
            let n = context.read(&mut header)?;
            if n == 0 {
                return Ok(None);
            }
            context.exact(&mut header[n..])?;
            let id = u32::from_le_bytes(header[0..4].try_into().unwrap());
            let size = u64::from_le_bytes(header[8..16].try_into().unwrap());
            let name_len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;
            if name_len > MAX_ZONE_BYTES || name_len % 2 != 0 {
                return Err("invalid named stream metadata".into());
            }
            let mut name = vec![0; name_len];
            context.exact(&mut name)?;
            let name = String::from_utf16(
                &name
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| "invalid named stream metadata")?;
            if id == 4 && name.eq_ignore_ascii_case(":Zone.Identifier:$DATA") {
                if size > MAX_ZONE_BYTES as u64 {
                    return Err("Internet zone metadata exceeds 64 KiB".into());
                }
                let mut bytes = vec![0; size as usize];
                context.exact(&mut bytes)?;
                return ZoneIdentifier::parse(bytes).map(Some);
            }
            if size > 0 {
                let (mut low, mut high) = (0, 0);
                if unsafe {
                    BackupSeek(
                        file.as_raw_handle(),
                        size as u32,
                        (size >> 32) as u32,
                        &mut low,
                        &mut high,
                        &mut context.raw,
                    )
                } == 0
                {
                    return Err(error("skip stream while reading"));
                }
                if (low as u64 | ((high as u64) << 32)) != size {
                    return Err("incomplete archive stream skip".into());
                }
            }
        }
        Err("too many archive alternate streams".into())
    }
    pub(super) fn write_zone(file: &File, bytes: &[u8]) -> Result<(), String> {
        if !named_streams(file)? {
            return Ok(());
        }
        let name: Vec<u8> = ":Zone.Identifier:$DATA"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut header = Vec::with_capacity(20 + name.len());
        header.extend_from_slice(&4u32.to_le_bytes()); // BACKUP_ALTERNATE_DATA
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        header.extend_from_slice(&(name.len() as u32).to_le_bytes());
        header.extend_from_slice(&name);
        let mut context = Context {
            file,
            raw: std::ptr::null_mut(),
            writing: true,
        };
        context.write(&header)?;
        context.write(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_valid_single_zone_id() {
        for bytes in [
            b"[ZoneTransfer]\r\nZoneId=3\r\n".to_vec(),
            b"[ZoneTransfer]\nZoneId=4\nReferrerUrl=https://example.com/".to_vec(),
        ] {
            assert!(ZoneIdentifier::parse(bytes).is_ok());
        }
        for text in [
            "ZoneId=3",
            "[ZoneTransfer]\nZoneId=9",
            "[ZoneTransfer]\nZoneId=3\nZoneId=0",
            "[other]\nZoneId=3",
        ] {
            assert!(ZoneIdentifier::parse(text.as_bytes().to_vec()).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn streams_roundtrip_through_write_only_handle_without_touching_data() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.zip");
        std::fs::write(&source, vec![7u8; 1024 * 1024]).unwrap();
        let zone = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/archive.zip\r\n";
        std::fs::write(format!("{}:Zone.Identifier", source.display()), zone).unwrap();
        let marker = from_archive(&source).unwrap().unwrap();
        assert_eq!(marker.bytes, zone);
        let out = dir.path().join("out.txt");
        let mut file = File::create(&out).unwrap();
        file.write_all(b"ordinary content").unwrap();
        apply_to_file(&file, Some(&marker)).unwrap();
        drop(file);
        assert_eq!(std::fs::read(&out).unwrap(), b"ordinary content");
        assert_eq!(
            std::fs::read(format!("{}:Zone.Identifier", out.display())).unwrap(),
            zone
        );
        assert_eq!(from_archive(&out).unwrap().unwrap().bytes, zone);
    }
    #[cfg(windows)]
    #[test]
    fn readonly_unmarked_and_unicode_sources() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.zip");
        std::fs::write(&source, b"contents").unwrap();
        assert!(from_archive(&source).unwrap().is_none());
        let unicode = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/中文.zip\r\n";
        for bytes in [
            unicode.as_bytes().to_vec(),
            [
                vec![0xff, 0xfe],
                unicode.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat(),
        ] {
            std::fs::write(format!("{}:Zone.Identifier", source.display()), &bytes).unwrap();
            let mut permissions = std::fs::metadata(&source).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&source, permissions).unwrap();
            let marker = from_archive(&source).unwrap().unwrap();
            assert_eq!(marker.bytes, bytes);
            let output = dir.path().join("out");
            let file = File::create(&output).unwrap();
            apply_to_file(&file, Some(&marker)).unwrap();
            drop(file);
            assert_eq!(
                std::fs::read(format!("{}:Zone.Identifier", output.display())).unwrap(),
                bytes
            );
            let mut permissions = std::fs::metadata(&source).unwrap().permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(&source, permissions).unwrap();
        }
    }
    #[cfg(windows)]
    #[test]
    fn unmarked_context_pins_source_until_last_clone_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("unmarked.zip");
        std::fs::write(&source, b"stable archive").unwrap();
        let context = archive_context(&source).unwrap();
        assert!(!context.is_marked());
        let clone = context.clone();
        drop(context);
        assert!(std::fs::write(&source, b"replacement").is_err());
        assert!(std::fs::rename(&source, dir.path().join("moved.zip")).is_err());
        assert!(std::fs::remove_file(&source).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"stable archive");
        drop(clone);
        std::fs::write(&source, b"released").unwrap();
    }
}
