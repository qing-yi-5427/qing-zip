//! Validation for local-copy BACKUP_EA_DATA, before writing a private output.
//!
//! This module does not set security descriptors, publish objects, or delete
//! sources. The caller must retain complete source/output readback verification.
//! FILE_FULL_EA_INFORMATION: MS-FSCC 2.4.16. Native BackupRead layout is covered
//! separately by a Windows fixture before enabling the caller's type-2 route.

use std::collections::BTreeSet;

// A bounded buffer allows validating the whole chain before BackupWrite sees it.
pub(super) const MAX_EA_BYTES: usize = 1024 * 1024;

pub(super) fn validate(
    stream_attributes: u32,
    stream_name_bytes: usize,
    payload: &[u8],
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    check()?;
    // Ordinary EA records have no stream name. Unknown backup attributes,
    // including security-bearing/modified-on-read data, need separate support.
    if stream_attributes != 0 || stream_name_bytes != 0 {
        return Err("暂不支持此扩展属性记录，源保持不变".into());
    }
    if payload.is_empty() || payload.len() > MAX_EA_BYTES {
        return Err("扩展属性记录大小无效".into());
    }
    let mut offset = 0usize;
    let mut names = BTreeSet::new();
    loop {
        check()?;
        let entry = payload.get(offset..).ok_or("扩展属性偏移无效")?;
        if entry.len() < 8 {
            return Err("扩展属性记录不完整".into());
        }
        let next = u32::from_le_bytes(entry[..4].try_into().unwrap()) as usize;
        let flags = entry[4];
        let name_len = entry[5] as usize;
        let value_len = u16::from_le_bytes(entry[6..8].try_into().unwrap()) as usize;
        let end = 8 + name_len + 1 + value_len;
        if !matches!(flags, 0 | 0x80) || name_len == 0 || end > entry.len() {
            return Err("扩展属性名称、标志或长度无效".into());
        }
        let name = &entry[8..8 + name_len];
        if entry[8 + name_len] != 0
            || name
                .iter()
                .any(|b| *b < 0x20 || b"\\/:*?\"<>|,+=[];".contains(b))
        {
            return Err("扩展属性名称无效".into());
        }
        // NTFS EA names compare without ASCII case; do not collapse entries.
        if !names.insert(name.iter().map(u8::to_ascii_uppercase).collect::<Vec<_>>()) {
            return Err("扩展属性名称重复".into());
        }
        if next == 0 {
            // Only final alignment padding is tolerated; it is not EA data.
            if entry.len() > ((end + 3) & !3) {
                return Err("扩展属性记录尾部无效".into());
            }
            return check();
        }
        if next % 4 != 0 || next < end || next >= entry.len() {
            return Err("扩展属性链无效".into());
        }
        offset = offset.checked_add(next).ok_or("扩展属性链溢出")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &[u8], value: &[u8], flags: u8) -> Vec<u8> {
        let mut data = vec![0; 8];
        data[4] = flags;
        data[5] = name.len() as u8;
        data[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());
        data.extend_from_slice(name);
        data.push(0);
        data.extend_from_slice(value);
        data
    }
    fn valid(data: &[u8]) -> bool {
        validate(0, 0, data, || Ok(())).is_ok()
    }

    #[test]
    fn ordinary_and_need_ea_keep_opaque_values() {
        assert!(valid(&entry(b"USER.META", &[0, 255, 3], 0)));
        assert!(valid(&entry(b"REQUIRED", &[5], 0x80)));
    }

    #[test]
    fn chained_entries_and_padding() {
        let mut first = entry(b"A", &[1], 0);
        first.resize((first.len() + 3) & !3, 0xcc);
        let next = first.len() as u32;
        first[..4].copy_from_slice(&next.to_le_bytes());
        first.extend(entry(b"B", &[2], 0x80));
        assert!(valid(&first));
        first[4] = 1;
        assert!(!valid(&first));
    }

    #[test]
    fn malformed_and_incomplete_records_are_rejected() {
        let good = entry(b"A", &[1, 2, 3], 0);
        for length in 0..good.len() {
            assert!(!valid(&good[..length]));
        }
        for next in [1u32, 8, 12, 0xffff_fffc] {
            let mut bad = good.clone();
            bad[..4].copy_from_slice(&next.to_le_bytes());
            assert!(!valid(&bad));
        }
        for name in [b"A:B".as_slice(), b"A\0B", b"A/B", b""] {
            assert!(!valid(&entry(name, &[1], 0)));
        }
        assert!(validate(2, 0, &good, || Ok(())).is_err());
        assert!(validate(0, 2, &good, || Ok(())).is_err());
    }

    #[test]
    fn duplicate_names_and_cancellation_are_rejected() {
        let mut first = entry(b"abc", &[1], 0);
        first.resize((first.len() + 3) & !3, 0);
        let next = first.len() as u32;
        first[..4].copy_from_slice(&next.to_le_bytes());
        first.extend(entry(b"ABC", &[2], 0));
        assert!(!valid(&first));
        let mut calls = 0;
        let result = validate(0, 0, &entry(b"A", &[1], 0), || {
            calls += 1;
            if calls == 3 {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), "cancelled");
    }

    #[cfg(windows)]
    #[test]
    fn native_backup_ea_record_matches_validated_layout() {
        use std::{ffi::c_void, fs::OpenOptions, os::windows::io::AsRawHandle};
        type Handle = *mut c_void;
        #[repr(C)]
        struct IoStatus {
            status: usize,
            information: usize,
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSetEaFile(file: Handle, status: *mut IoStatus, data: *const u8, size: u32) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn BackupRead(
                file: Handle,
                data: *mut u8,
                size: u32,
                read: *mut u32,
                abort: i32,
                security: i32,
                context: *mut Handle,
            ) -> i32;
            fn BackupWrite(
                file: Handle,
                data: *const u8,
                size: u32,
                written: *mut u32,
                abort: i32,
                security: i32,
                context: *mut Handle,
            ) -> i32;
        }
        struct Reader<'a> {
            file: &'a std::fs::File,
            context: Handle,
        }
        impl Reader<'_> {
            fn read(&mut self, bytes: &mut [u8]) -> usize {
                let mut n = 0;
                assert_ne!(
                    unsafe {
                        BackupRead(
                            self.file.as_raw_handle(),
                            bytes.as_mut_ptr(),
                            bytes.len() as u32,
                            &mut n,
                            0,
                            0,
                            &mut self.context,
                        )
                    },
                    0,
                    "{}",
                    std::io::Error::last_os_error()
                );
                n as usize
            }
            fn exact(&mut self, mut bytes: &mut [u8]) {
                while !bytes.is_empty() {
                    let n = self.read(bytes);
                    assert!(n > 0, "native backup unexpectedly truncated");
                    bytes = &mut bytes[n..];
                }
            }
        }
        impl Drop for Reader<'_> {
            fn drop(&mut self) {
                let mut n = 0;
                unsafe {
                    BackupRead(
                        self.file.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        &mut n,
                        1,
                        0,
                        &mut self.context,
                    );
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        for flags in [0, 0x80] {
            let path = temp.path().join(format!("ea-{flags}.bin"));
            let file = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(path)
                .unwrap();
            let fixture = entry(b"QING.ZIP.TEST", &[0, 17, 255], flags);
            let mut status = IoStatus {
                status: 0,
                information: 0,
            };
            let result = unsafe {
                NtSetEaFile(
                    file.as_raw_handle(),
                    &mut status,
                    fixture.as_ptr(),
                    fixture.len() as u32,
                )
            };
            assert_eq!(result, 0, "test volume must support EA: {result:#x}");
            let mut reader = Reader {
                file: &file,
                context: std::ptr::null_mut(),
            };
            let mut seen = false;
            loop {
                let mut header = [0; 20];
                let n = reader.read(&mut header);
                if n == 0 {
                    break;
                }
                reader.exact(&mut header[n..]);
                let id = u32::from_le_bytes(header[..4].try_into().unwrap());
                let attributes = u32::from_le_bytes(header[4..8].try_into().unwrap());
                let size = u64::from_le_bytes(header[8..16].try_into().unwrap());
                let name_size = u32::from_le_bytes(header[16..].try_into().unwrap()) as usize;
                assert!(size <= MAX_EA_BYTES as u64 && name_size <= 65536);
                let mut name = vec![0; name_size];
                reader.exact(&mut name);
                let mut payload = vec![0; size as usize];
                reader.exact(&mut payload);
                if id == 2 {
                    assert!(!seen);
                    seen = true;
                    validate(attributes, name_size, &payload, || Ok(())).unwrap();
                    assert_eq!(payload, fixture);
                    let target = OpenOptions::new()
                        .create_new(true)
                        .read(true)
                        .write(true)
                        .open(temp.path().join(format!("copied-{flags}.bin")))
                        .unwrap();
                    let mut context = std::ptr::null_mut();
                    for data in [header.as_slice(), payload.as_slice()] {
                        let mut written = 0;
                        assert_ne!(
                            unsafe {
                                BackupWrite(
                                    target.as_raw_handle(),
                                    data.as_ptr(),
                                    data.len() as u32,
                                    &mut written,
                                    0,
                                    0,
                                    &mut context,
                                )
                            },
                            0
                        );
                        assert_eq!(written as usize, data.len());
                    }
                    let mut written = 0;
                    unsafe {
                        BackupWrite(
                            target.as_raw_handle(),
                            std::ptr::null(),
                            0,
                            &mut written,
                            1,
                            0,
                            &mut context,
                        );
                    }
                    let mut copied = Reader {
                        file: &target,
                        context: std::ptr::null_mut(),
                    };
                    let mut verified = false;
                    loop {
                        let mut h = [0; 20];
                        let n = copied.read(&mut h);
                        if n == 0 {
                            break;
                        }
                        copied.exact(&mut h[n..]);
                        let len = u64::from_le_bytes(h[8..16].try_into().unwrap());
                        let name_len = u32::from_le_bytes(h[16..].try_into().unwrap()) as usize;
                        assert!(len <= MAX_EA_BYTES as u64 && name_len <= 65536);
                        let mut body = vec![0; name_len + len as usize];
                        copied.exact(&mut body);
                        if u32::from_le_bytes(h[..4].try_into().unwrap()) == 2 {
                            assert_eq!(h, header);
                            assert_eq!(body, payload);
                            verified = true;
                        }
                    }
                    assert!(
                        verified,
                        "BackupWrite output must preserve EA bytes and flags"
                    );
                }
            }
            assert!(seen, "BackupRead must expose the EA record");
        }
    }
}
