//! Sparse layout preservation for local-copy private streams.
//! Full logical bytes remain covered by the caller's stream hash and readback.
use super::JobControl;
use std::{
    ffi::c_void,
    fs::File,
    os::windows::{fs::MetadataExt, io::AsRawHandle},
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        file: *mut c_void,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub eof: u64,
    pub ranges: Vec<(u64, u64)>,
}

pub(super) fn block_end(
    eof: u64,
    previous: u64,
    start: u64,
    backup_size: u64,
    attributes: u32,
) -> Result<u64, String> {
    if attributes != 8 || backup_size < 8 {
        return Err("稀疏数据块无效".into());
    }
    let end = start.checked_add(backup_size - 8).ok_or("稀疏数据块溢出")?;
    if start < previous || end > eof {
        return Err("稀疏数据块越界或重叠".into());
    }
    Ok(end)
}

pub(super) fn query(file: &File, ctl: &JobControl) -> Result<Option<Layout>, String> {
    ctl.check()?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if meta.file_attributes() & 0x200 == 0 {
        return Ok(None);
    }
    let eof = meta.len();
    if eof > i64::MAX as u64 {
        return Err("稀疏文件长度超出范围".into());
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < eof {
        ctl.check()?;
        let input = [start as i64, (eof - start) as i64];
        let mut output = [0i64; 512];
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x940cf,
                input.as_ptr().cast(),
                16,
                output.as_mut_ptr().cast(),
                std::mem::size_of_val(&output) as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        let error = std::io::Error::last_os_error();
        if ok == 0 && error.raw_os_error() != Some(234) {
            return Err(format!("无法读取稀疏文件区间：{error}"));
        }
        if returned as usize > std::mem::size_of_val(&output) || returned % 16 != 0 {
            return Err("稀疏文件区间数据无效".into());
        }
        let previous = start;
        for pair in output[..returned as usize / 8].chunks_exact(2) {
            if pair[0] < 0 || pair[1] <= 0 {
                return Err("稀疏文件区间无效".into());
            }
            let offset = pair[0] as u64;
            let length = pair[1] as u64;
            let end = offset.checked_add(length).ok_or("稀疏文件区间溢出")?;
            if offset < start || end > eof {
                return Err("稀疏文件区间越界或重叠".into());
            }
            ranges.push((offset, length));
            start = end;
            if ranges.len() > 1_000_000 {
                return Err("稀疏文件区间过多".into());
            }
        }
        if ok != 0 {
            break;
        }
        if start <= previous {
            return Err("稀疏文件区间查询未前进".into());
        }
    }
    Ok(Some(Layout { eof, ranges }))
}

pub(super) fn restore(file: &File, layout: &Layout, ctl: &JobControl) -> Result<(), String> {
    ctl.check()?;
    #[cfg(test)]
    if FAIL_RESTORE.with(|failure| failure.replace(false)) {
        return Err("目标无法保留稀疏存储属性，源保持不变：不支持此请求 (os error 50)".into());
    }
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x900c4,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "目标无法保留稀疏存储属性，源保持不变：{}",
            std::io::Error::last_os_error()
        ));
    }
    if file.metadata().map_err(|e| e.to_string())?.len() != layout.eof {
        return Err("稀疏文件长度核对失败".into());
    }
    let mut start = 0;
    for &(offset, length) in &layout.ranges {
        zero(file, start, offset, ctl)?;
        start = offset + length;
    }
    zero(file, start, layout.eof, ctl)?;
    if query(file, ctl)?.as_ref() != Some(layout) {
        return Err("目标稀疏区间核对失败，源保持不变".into());
    }
    Ok(())
}
#[cfg(test)]
thread_local! {pub(super) static FAIL_RESTORE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
pub(super) fn zero(file: &File, start: u64, end: u64, ctl: &JobControl) -> Result<(), String> {
    ctl.check()?;
    if start == end {
        return Ok(());
    }
    let input = [start as i64, end as i64];
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x980c8,
            input.as_ptr().cast(),
            16,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "目标无法保留稀疏空洞，源保持不变：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_blocks_require_bounded_ordered_offsets() {
        assert_eq!(block_end(100, 0, 20, 18, 8).unwrap(), 30);
        assert_eq!(block_end(100, 30, 100, 8, 8).unwrap(), 100);
        for args in [
            (100, 30, 29, 9, 8),
            (100, 0, 99, 10, 8),
            (100, 0, 0, 7, 8),
            (100, 0, 0, 8, 0),
            (u64::MAX, 0, u64::MAX, 9, 8),
        ] {
            assert!(block_end(args.0, args.1, args.2, args.3, args.4).is_err());
        }
    }
}
