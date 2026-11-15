//! Handle-relative NTFS compression state for local-copy private outputs.
use std::{
    ffi::c_void,
    fs::File,
    os::windows::{
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
};
type Handle = *mut c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        file: Handle,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
        returned: *mut u32,
        overlapped: Handle,
    ) -> i32;
}
pub(super) fn get(file: &File) -> Result<u16, String> {
    let mut value = 0u16;
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x9003c,
            std::ptr::null(),
            0,
            (&mut value as *mut u16).cast(),
            2,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(1 | 50))
            && file
                .metadata()
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x800
                == 0
        {
            return Ok(0);
        }
        return Err(format!("无法读取压缩存储属性：{error}"));
    }
    if returned != 2 || !matches!(value, 0 | 2) {
        return Err("不支持此压缩存储格式".into());
    }
    Ok(value)
}
pub(super) fn set(file: &File, value: u16) -> Result<(), String> {
    if !matches!(value, 0 | 2) {
        return Err("不支持此压缩存储格式".into());
    }
    if get(file)? == value {
        return Ok(());
    }
    #[cfg(test)]
    if FAIL_SET.with(|value| value.replace(false)) {
        return Err("目标无法保留压缩存储属性，源保持不变：不支持此请求 (os error 50)".into());
    }
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x9c040,
            (&value as *const u16).cast(),
            2,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "目标无法保留压缩存储属性，源保持不变：{}",
            std::io::Error::last_os_error()
        ));
    }
    if get(file)? != value {
        return Err("目标压缩存储属性核对失败".into());
    }
    Ok(())
}
#[cfg(test)]
thread_local! {pub(super) static FAIL_SET:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
pub(super) fn stream(file: &File, name: &[u16], write: bool) -> Result<File, String> {
    #[repr(C)]
    struct Unicode {
        length: u16,
        max: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct Attr {
        length: u32,
        root: Handle,
        name: *mut Unicode,
        attributes: u32,
        sd: Handle,
        sqos: Handle,
    }
    #[repr(C)]
    struct Status {
        status: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            out: *mut Handle,
            access: u32,
            attrs: *const Attr,
            status: *mut Status,
            size: *const i64,
            attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            ea: Handle,
            ea_len: u32,
        ) -> i32;
    }
    let bytes = u16::try_from(name.len() * 2).map_err(|_| "数据流名称过长")?;
    let mut text = name.to_vec();
    let mut unicode = Unicode {
        length: bytes,
        max: bytes,
        buffer: text.as_mut_ptr(),
    };
    let mut attrs = Attr {
        length: std::mem::size_of::<Attr>() as u32,
        root: file.as_raw_handle(),
        name: &mut unicode,
        attributes: 0x40,
        sd: std::ptr::null_mut(),
        sqos: std::ptr::null_mut(),
    };
    let mut status = Status {
        status: 0,
        information: 0,
    };
    let mut out = std::ptr::null_mut();
    let result = unsafe {
        NtCreateFile(
            &mut out,
            if write { 0xC0100000 } else { 0x80100000 },
            &mut attrs,
            &mut status,
            std::ptr::null(),
            0,
            7,
            1,
            0x200020,
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(format!("无法固定备用数据流：{result:#x}"));
    }
    Ok(unsafe { File::from_raw_handle(out) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::OpenOptionsExt};
    #[test]
    fn native_main_ads_and_directory_have_independent_states() {
        let temp = tempfile::tempdir().unwrap();
        for directory in [false, true] {
            let path = temp
                .path()
                .join(if directory { "directory" } else { "file" });
            if directory {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, b"main").unwrap();
            }
            fs::write(format!("{}:custom", path.display()), b"ads").unwrap();
            let file = File::options()
                .read(true)
                .write(true)
                .share_mode(7)
                .custom_flags(0x02000000)
                .open(&path)
                .unwrap();
            let ads = stream(
                &file,
                &":custom:$DATA".encode_utf16().collect::<Vec<_>>(),
                true,
            )
            .unwrap();
            set(&file, 2).unwrap();
            set(&ads, 0).unwrap();
            assert_eq!(get(&file).unwrap(), 2);
            assert_eq!(get(&ads).unwrap(), 0);
            set(&file, 0).unwrap();
            set(&ads, 2).unwrap();
            assert_eq!(get(&file).unwrap(), 0);
            assert_eq!(get(&ads).unwrap(), 2);
        }
    }
}
