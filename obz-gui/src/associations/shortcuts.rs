use super::{RawValue, Registry};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows::{
    core::GUID,
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE},
        Storage::FileSystem::{
            FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
            BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_DISPOSITION_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        },
        System::{Com::CoTaskMemFree, Registry::REG_BINARY},
        UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    },
};

const PRODUCT: &str = if cfg!(target_arch = "aarch64") {
    "qing-zip-arm64"
} else {
    "qing-zip"
};
// The installer creates only small shortcut templates. Never read an unbounded
// replacement on the Desktop or Start menu when deciding ownership.
const MAX_LINK: usize = 32 * 1024;
const SLOTS: [&str; 3] = ["start", "uninstall", "desktop"];
fn known_folder(id: &GUID) -> Result<PathBuf, String> {
    let raw =
        unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.map_err(|e| e.to_string())?;
    let path = PathBuf::from(unsafe { OsString::from_wide(raw.as_wide()) });
    unsafe {
        CoTaskMemFree(Some(raw.0.cast()));
    }
    Ok(path)
}
fn slot_path(slot: &str) -> Result<PathBuf, String> {
    match slot {
        "start" => Ok(known_folder(&FOLDERID_Programs)?
            .join(PRODUCT)
            .join("qing-zip.lnk")),
        "uninstall" => Ok(known_folder(&FOLDERID_Programs)?
            .join(PRODUCT)
            .join("卸载.lnk")),
        "desktop" => Ok(known_folder(&FOLDERID_Desktop)?.join(format!("{PRODUCT}.lnk"))),
        _ => Err("unknown shortcut slot".into()),
    }
}
fn record_key() -> String {
    format!(r"Software\{PRODUCT}\Installer\Shortcuts")
}
fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn remove_file(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            handle(file),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    }?;
    Ok(())
}
fn open_options(write: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    let access = GENERIC_READ.0 | DELETE.0 | if write { GENERIC_WRITE.0 } else { 0 };
    options
        .read(true)
        .write(write)
        .access_mode(access)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .create_new(write);
    options
}
fn record(owner: &Path, link: &Path, file: &File, bytes: &[u8]) -> io::Result<RawValue> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle(file), &mut info) }?;
    let mut data = Vec::new();
    for path in [owner, link] {
        data.extend(
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes),
        );
    }
    for value in [
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ] {
        data.extend(value.to_le_bytes());
    }
    data.extend(bytes);
    if data.len() > 65536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shortcut ownership record too large",
        ));
    }
    Ok(RawValue {
        kind: REG_BINARY,
        data,
    })
}
fn install(
    reg: &Registry,
    owner: &Path,
    slot: &str,
    link: &Path,
    template: &Path,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    File::open(template)
        .and_then(|f| f.take(MAX_LINK as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_LINK || !bytes.starts_with(&[0x4c, 0, 0, 0, 1, 0x14, 2, 0]) {
        return Err("invalid installer shortcut template".into());
    }
    let mut file = match open_options(true).open(link) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let mut publish = || -> Result<(), String> {
        file.write_all(&bytes)
            .and_then(|_| file.flush())
            .map_err(|e| e.to_string())?;
        let stamp = record(owner, link, &file, &bytes).map_err(|e| e.to_string())?;
        reg.write(&record_key(), slot, Some(&stamp))
    };
    if let Err(e) = publish() {
        remove_file(&file).map_err(|cleanup| format!("{e}; {cleanup}"))?;
        return Err(e);
    }
    Ok(())
}
fn remove(reg: &Registry, owner: &Path, slot: &str, link: &Path) -> Result<bool, String> {
    let Some(stamp) = reg.read(&record_key(), slot)? else {
        return Ok(false);
    };
    let mut file = match open_options(false).open(link) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || metadata.len() > MAX_LINK as u64
    {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let actual = record(owner, link, &file, &bytes).map_err(|e| e.to_string())?;
    if actual != stamp {
        return Ok(false);
    }
    remove_file(&file).map_err(|e| e.to_string())?;
    Ok(true)
}
pub(crate) fn configure(args: &[OsString]) -> Option<Result<(), String>> {
    match args {
        [command, slot, template] if command == "--install-owned-shortcut" => Some((|| {
            let owner = std::env::current_exe().map_err(|e| e.to_string())?;
            let slot = slot.to_str().ok_or("invalid shortcut slot")?;
            install(
                &Registry::user(),
                &owner,
                slot,
                &slot_path(slot)?,
                Path::new(template),
            )
        })()),
        [command] if command == "--uninstall-owned-shortcuts" => Some((|| {
            let reg = Registry::user();
            let owner = std::env::current_exe().map_err(|e| e.to_string())?;
            for slot in SLOTS {
                remove(&reg, &owner, slot, &slot_path(slot)?)?;
            }
            Ok(())
        })()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::associations::tests::TestRegistry;

    #[test]
    fn shortcut_cleanup_checks_content_identity_install_path_and_ownership() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("shortcut.lnk");
        let staged = dir.path().join("template.lnk");
        let installed = dir.path().join("installed.exe");
        let portable = dir.path().join("portable.exe");
        let template = [0x4c, 0, 0, 0, 1, 0x14, 2, 0, 1, 2, 3, 4].as_slice();
        std::fs::write(&staged, template).unwrap();

        std::fs::write(&link, b"user shortcut").unwrap();
        install(reg, &installed, "start", &link, &staged).unwrap();
        assert!(!remove(reg, &installed, "start", &link).unwrap());
        assert_eq!(std::fs::read(&link).unwrap(), b"user shortcut");
        std::fs::remove_file(&link).unwrap();

        install(reg, &installed, "start", &link, &staged).unwrap();
        install(reg, &installed, "start", &link, &staged).unwrap();
        assert!(!remove(reg, &portable, "start", &link).unwrap());
        assert!(remove(reg, &installed, "start", &link).unwrap());
        assert!(!link.exists());

        install(reg, &installed, "start", &link, &staged).unwrap();
        std::fs::write(&link, b"portable takeover").unwrap();
        assert!(!remove(reg, &installed, "start", &link).unwrap());
        assert_eq!(std::fs::read(&link).unwrap(), b"portable takeover");

        // Keep the original file live under a new name so the file identifier
        // cannot be recycled while testing an identical-byte replacement.
        std::fs::rename(&link, dir.path().join("old.lnk")).unwrap();
        std::fs::write(&link, template).unwrap();
        assert!(!remove(reg, &installed, "start", &link).unwrap());
    }
}
