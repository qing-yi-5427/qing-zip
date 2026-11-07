//! Format policy uses the same four machine values as NanaZip. An operation
//! keeps one snapshot for nested archive calls, including its verified backend.
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::OnceLock,
};

const VALUES: [&str; 4] = [
    "AllowedHandlers",
    "BlockedHandlers",
    "AllowedCodecs",
    "BlockedCodecs",
];
#[derive(Default, PartialEq, Eq)]
struct Rules {
    sets: [BTreeSet<String>; 4],
    // A malformed/oversized value cannot authorize an independent decoder.
    opaque: bool,
}
enum Value {
    Missing,
    Strings(Vec<u16>),
    Opaque,
}
impl Rules {
    fn read(mut get: impl FnMut(&str) -> Value) -> Self {
        let mut result = Self::default();
        for (index, name) in VALUES.iter().enumerate() {
            match get(name) {
                Value::Missing => (),
                Value::Opaque => result.opaque = true,
                Value::Strings(units) => {
                    let mut remaining = units.as_slice();
                    while remaining.first().is_some_and(|&first| first != 0) {
                        let Some(end) = remaining.iter().position(|&unit| unit == 0) else {
                            result.opaque = true;
                            break;
                        };
                        match String::from_utf16(&remaining[..end]) {
                            Ok(value) => {
                                result.sets[index].insert(value);
                            }
                            Err(_) => result.opaque = true,
                        }
                        remaining = &remaining[end + 1..];
                        if remaining.is_empty() {
                            result.opaque = true;
                        }
                    }
                }
            }
        }
        result
    }
    fn restricted(&self) -> bool {
        self.opaque || self.sets.iter().any(|s| !s.is_empty())
    }
    fn allows(&self, kind: usize, name: &str) -> bool {
        let allow = &self.sets[kind];
        let block = &self.sets[kind + 1];
        !block.contains(name) && (allow.is_empty() || allow.contains(name))
    }
    fn token(&self) -> String {
        if !self.restricted() {
            return "default".into();
        }
        // Unknown policies always invalidate preview cache entries.
        if self.opaque {
            static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            return format!(
                "opaque-{}",
                EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
        }
        let mut hash = Sha256::new();
        for set in &self.sets {
            hash.update((set.len() as u64).to_le_bytes());
            for name in set {
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name.as_bytes());
            }
        }
        crate::checksum::hex(&hash.finalize())
    }
}

#[cfg(windows)]
fn read_machine() -> Rules {
    // HKEY_LOCAL_MACHINE in the 64-bit view.
    read_machine_at(-2147483646isize as *mut std::ffi::c_void)
}

#[cfg(windows)]
fn read_machine_at(root: *mut std::ffi::c_void) -> Rules {
    use std::ffi::c_void;
    type Key = *mut c_void;
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            root: Key,
            name: *const u16,
            options: u32,
            access: u32,
            key: *mut Key,
        ) -> i32;
        fn RegQueryValueExW(
            key: Key,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            length: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: Key) -> i32;
    }
    struct Guard(Key);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let key_name: Vec<_> = r"Software\Policies\M2Team\NanaZip"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut key = std::ptr::null_mut();
    let null = std::ptr::null_mut();
    // HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY; never consult HKCU or environment.
    let status = unsafe { RegOpenKeyExW(root, key_name.as_ptr(), 0, 0x20119, &mut key) };
    if status != 0 {
        return Rules {
            opaque: !matches!(status, 2 | 3),
            ..Rules::default()
        };
    }
    let key = Guard(key);
    Rules::read(|name| {
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let (mut kind, mut length) = (0u32, 0u32);
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                wide.as_ptr(),
                null,
                &mut kind,
                std::ptr::null_mut(),
                &mut length,
            )
        };
        match status {
            0 if kind == 7 => (),
            // K7 ignores an absent value or a value of another registry type.
            0 | 2 => return Value::Missing,
            _ => return Value::Opaque,
        }
        if length > 1024 * 1024 || length % 2 != 0 {
            return Value::Opaque;
        }
        let mut units = vec![0u16; length as usize / 2];
        if length == 0 {
            return Value::Missing;
        }
        let mut actual = length;
        if unsafe {
            RegQueryValueExW(
                key.0,
                wide.as_ptr(),
                null,
                &mut kind,
                units.as_mut_ptr().cast(),
                &mut actual,
            )
        } != 0
        {
            return Value::Opaque;
        }
        if kind != 7 || actual != length {
            return Value::Opaque;
        }
        Value::Strings(units)
    })
}
#[cfg(not(windows))]
fn read_machine() -> Rules {
    Rules::default()
}

#[cfg(all(windows, target_arch = "x86_64", target_env = "msvc"))]
#[no_mangle]
extern "C" fn qing_machine_policy_restricts_native() -> i32 {
    i32::from(std::panic::catch_unwind(|| read_machine().restricted()).unwrap_or(true))
}

struct Backend {
    exe: PathBuf,
    _pins: Vec<crate::engine_stream::PinnedSource>,
}
struct Operation {
    rules: Rules,
    token: OnceLock<String>,
    backend: OnceLock<Result<Backend, String>>,
    #[cfg(windows)]
    notifications: Option<Result<Watch, String>>,
}
impl Operation {
    fn new(rules: Rules) -> Self {
        Self {
            rules,
            token: OnceLock::new(),
            backend: OnceLock::new(),
            #[cfg(windows)]
            notifications: None,
        }
    }
    fn machine() -> Self {
        let rules = read_machine();
        #[cfg(windows)]
        if rules.restricted() && !rules.opaque && !rules.allows(0, "ZealFS") {
            // Nana rejects -stxZealFS when policy removed that handler. Arm
            // the notification before taking the authoritative second snapshot:
            // a newly enabled handler must never decode without our guard.
            let notifications = Watch::arm();
            let mut op = Self::new(read_machine());
            op.notifications = Some(notifications);
            return op;
        }
        Self::new(rules)
    }
}
#[cfg(windows)]
struct Watch {
    key: *mut std::ffi::c_void,
    event: std::os::windows::io::OwnedHandle,
}
#[cfg(windows)]
impl Watch {
    fn arm() -> Result<Self, String> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        type H = *mut std::ffi::c_void;
        #[link(name = "advapi32")]
        extern "system" {
            fn RegOpenKeyExW(
                root: H,
                name: *const u16,
                options: u32,
                access: u32,
                key: *mut H,
            ) -> i32;
            fn RegNotifyChangeKeyValue(
                key: H,
                subtree: i32,
                flags: u32,
                event: H,
                asynchronous: i32,
            ) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateEventW(security: H, manual: i32, initial: i32, name: *const u16) -> H;
        }
        let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err("cannot watch the machine format policy".into());
        }
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        let mut key = std::ptr::null_mut();
        let name: Vec<_> = r"Software\Policies".encode_utf16().chain(Some(0)).collect();
        if unsafe { RegOpenKeyExW(-2147483646isize as H, name.as_ptr(), 0, 0x110, &mut key) } != 0 {
            return Err("cannot watch the machine format policy".into());
        }
        let watch = Self { key, event };
        if unsafe {
            RegNotifyChangeKeyValue(watch.key, 1, 0x1000000d, watch.event.as_raw_handle(), 1)
        } != 0
        {
            return Err("cannot watch the machine format policy".into());
        }
        Ok(watch)
    }
    fn unchanged(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" {
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        }
        unsafe { WaitForSingleObject(self.event.as_raw_handle(), 0) == 258 }
    }
}
#[cfg(windows)]
impl Drop for Watch {
    fn drop(&mut self) {
        #[link(name = "advapi32")]
        extern "system" {
            fn RegCloseKey(key: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            RegCloseKey(self.key);
        }
    }
}
thread_local! { static ACTIVE: RefCell<Option<(Rc<Operation>, usize)>> = const { RefCell::new(None) }; }

/// Read-only snapshot for the calling thread's current archive operation.
pub struct Scope {
    operation: Rc<Operation>,
}
pub fn scope() -> Scope {
    ACTIVE.with(|active| {
        let mut slot = active.borrow_mut();
        let (operation, depth) = slot.get_or_insert_with(|| (Rc::new(Operation::machine()), 0));
        *depth += 1;
        Scope {
            operation: operation.clone(),
        }
    })
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            let mut slot = active.borrow_mut();
            if let Some((_, depth)) = slot.as_mut() {
                *depth -= 1;
                if *depth == 0 {
                    *slot = None;
                }
            }
        });
    }
}
impl Scope {
    pub fn restricted(&self) -> bool {
        self.operation.rules.restricted()
    }
    pub fn cache_token(&self) -> &str {
        self.operation
            .token
            .get_or_init(|| self.operation.rules.token())
    }
    /// A preview request can wait behind another decoder or hit a plaintext
    /// cache. Re-read the effective policy just before releasing its result.
    pub fn validate_cached_access(&self) -> Result<(), String> {
        if self.operation.rules.opaque || self.operation.rules != read_machine() {
            return Err(
                "machine format policy changed or is unreadable; reopen the preview".into(),
            );
        }
        self.unchanged()
    }
    pub fn unchanged(&self) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(result) = &self.operation.notifications {
            match result {
                Ok(watch) if watch.unchanged() => (),
                Ok(_) => return Err(
                    "machine policy changed during this operation; retry with the current settings"
                        .into(),
                ),
                Err(error) => return Err(error.clone()),
            }
        }
        Ok(())
    }
    pub(crate) fn handler_may_be_enabled(&self, name: &str) -> bool {
        self.operation.rules.opaque || self.operation.rules.allows(0, name)
    }
    pub(crate) fn official_executable(&self) -> Result<PathBuf, String> {
        if self.operation.rules.opaque {
            return Err("cannot reliably read the NanaZip machine format policy".into());
        }
        self.unchanged()?;
        match self.operation.backend.get_or_init(verify_backend) {
            Ok(backend) => Ok(backend.exe.clone()),
            Err(error) => Err(error.clone()),
        }
    }
}
pub(crate) fn native(feature: &str) -> Result<Scope, String> {
    let scope = scope();
    if scope.restricted() {
        Err(format!("NanaZip machine format policy requires its own archive engine; {feature} uses an independent implementation"))
    } else {
        Ok(scope)
    }
}

fn verify_backend() -> Result<Backend, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = current
        .parent()
        .ok_or("application directory unavailable")?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("build source directory unavailable")?;
    // Cargo target executables can use the repository's bundled runtime. An
    // installed copy with a missing runtime must never hunt for an installed7z.
    let root = if !app.join("runtime").exists() && current.starts_with(workspace) {
        workspace
    } else {
        app
    };
    verify_backend_at(&root.join("runtime/nanazip"))
}
fn verify_backend_at(root: &Path) -> Result<Backend, String> {
    #[derive(serde::Deserialize)]
    struct Provenance {
        files: std::collections::BTreeMap<String, String>,
    }
    let published: Provenance =
        serde_json::from_str(include_str!("../../runtime/nanazip/provenance.json"))
            .map_err(|e| e.to_string())?;
    let mut pins = Vec::new();
    for (name, expected) in published.files {
        let mut pin = crate::engine_stream::pin(&root.join(name)).map_err(|_| {
            "NanaZip machine format policy requires the bundled official runtime".to_owned()
        })?;
        let mut hash = Sha256::new();
        if pin.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
            return Err("bundled NanaZip runtime identity mismatch".into());
        }
        let mut block = [0u8; 64 * 1024];
        loop {
            let n = pin.read(&mut block).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&block[..n]);
        }
        if crate::checksum::hex(&hash.finalize()) != expected {
            return Err("bundled NanaZip runtime identity mismatch".into());
        }
        pins.push(pin);
    }
    Ok(Backend {
        exe: root.join("NanaZip.Universal.Console.exe"),
        _pins: pins,
    })
}

#[cfg(test)]
pub(crate) fn with_rules<T>(sets: [&[&str]; 4], run: impl FnOnce() -> T) -> T {
    let rules = Rules {
        sets: sets.map(|s| s.iter().map(|s| (*s).to_owned()).collect()),
        opaque: false,
    };
    let old = ACTIVE.with(|slot| slot.replace(Some((Rc::new(Operation::new(rules)), 1))));
    struct Restore(Option<(Rc<Operation>, usize)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|slot| slot.replace(self.0.take()));
        }
    }
    let _restore = Restore(old);
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::{self, EngineOptions},
        operations,
    };
    use std::io::Write;

    fn multi(values: &[&str]) -> Value {
        let mut utf16 = Vec::new();
        for value in values {
            utf16.extend(value.encode_utf16());
            utf16.push(0);
        }
        utf16.push(0);
        Value::Strings(utf16)
    }

    #[test]
    fn exact_multi_string_semantics_and_independent_codec_sets() {
        let rules = Rules::read(|name| match name {
            "AllowedHandlers" => multi(&["zip", "Zip", " zip ", "圧縮", "zip"]),
            "BlockedHandlers" => multi(&["Zip"]),
            "AllowedCodecs" => multi(&[]),
            "BlockedCodecs" => multi(&["LZMA2"]),
            _ => unreachable!(),
        });
        assert!(rules.restricted());
        for allowed in ["zip", " zip ", "圧縮"] {
            assert!(rules.allows(0, allowed));
        }
        for blocked in ["Zip", "ZIP", "7z", "*"] {
            assert!(!rules.allows(0, blocked));
        }
        assert!(rules.allows(2, "lzma2"));
        assert!(!rules.allows(2, "LZMA2"));
        assert!(rules.allows(2, "anything else"));
        assert_eq!(rules.sets[0].len(), 4);
        assert!(!Rules::read(|_| Value::Missing).restricted());
        assert!(!Rules::read(|_| Value::Strings(vec![0, 0, 65, 0, 0])).restricted());
        assert!(Rules::read(|_| Value::Strings(vec![65])).opaque);
        assert!(Rules::read(|_| Value::Strings(vec![0xd800, 0, 0])).opaque);
        assert!(Rules::read(|_| Value::Opaque).restricted());
    }

    #[test]
    fn snapshot_tokens_change_on_case_list_and_restriction_transitions() {
        let token = |sets| {
            with_rules(sets, || {
                let a = scope();
                let b = scope();
                assert_eq!(a.cache_token(), b.cache_token());
                a.cache_token().to_owned()
            })
        };
        let clear = token([&[], &[], &[], &[]]);
        let a = token([&["7z"], &[], &[], &[]]);
        let b = token([&["7Z"], &[], &[], &[]]);
        let c = token([&[], &[], &["7z"], &[]]);
        assert_ne!(clear, a);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a, token([&["7z", "7z"], &[], &[], &[]]));
    }

    #[cfg(windows)]
    #[test]
    fn an_unreadable_registry_root_cannot_enable_any_archive_backend() {
        // Win32 rejects this root with ERROR_INVALID_HANDLE, independently
        // of the policy a developer or administrator has on their machine.
        let rules = read_machine_at(std::ptr::null_mut());
        assert!(rules.opaque);
        let scope = Scope {
            operation: Rc::new(Operation::new(rules)),
        };
        assert!(scope.restricted());
        assert!(scope
            .official_executable()
            .unwrap_err()
            .contains("machine format policy"));
    }

    #[test]
    fn independent_adapters_refuse_before_input_or_output_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let absent = tmp.path().join("missing");
        let output = tmp.path().join("untouched");
        let path = absent.to_str().unwrap();
        let dest = output.to_str().unwrap();
        let opts = EngineOptions::default();
        with_rules([&[], &[], &[], &["arbitrary blocked codec"]], || {
            let expect_policy = |result: Result<(), String>| {
                assert!(result.unwrap_err().contains("machine format policy"))
            };
            expect_policy(crate::rare::list(path, &opts).map(drop));
            expect_policy(crate::legacy::extract(path, dest, &opts).map(drop));
            expect_policy(crate::legacy_worker::list(path, &opts).map(drop));
            expect_policy(crate::envelopes::preview(path, "test", &opts).map(drop));
            expect_policy(crate::korean::test(path, &opts));
            expect_policy(crate::installer::recognize(path).map(drop));
            expect_policy(crate::setup::open(&[]).map(drop));
            expect_policy(crate::sevenz::parse_archive(&[]).map(drop));
            expect_policy(crate::zipfmt::read_central_dir(&[], 0, 0).map(drop));
            expect_policy(crate::zipfmt::create_zip(dest, &[], false).map(drop));
            expect_policy(crate::archive_comment::get(path).map(drop));
            expect_policy(crate::archive_comment::set(path, "x", &opts));
            expect_policy(crate::services::repair_zip(path, dest, 4096).map(drop));
            expect_policy(crate::services::create_iso(&[], dest).map(drop));
            for format in ["sfx", "zipx", "lzh", "iso"] {
                expect_policy(engine::create(dest, &[], format, &opts).map(drop));
            }
            expect_policy(
                operations::execute(operations::Request {
                    operation: "recover".into(),
                    path: path.into(),
                    ..Default::default()
                })
                .map(drop),
            );
        });
        assert!(!output.exists());
    }

    #[test]
    fn missing_or_replaced_bundled_backend_never_selects_a_compatibility_executable() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("7z.exe"), b"unrelated").unwrap();
        assert!(verify_backend_at(tmp.path()).is_err());
        std::fs::write(
            tmp.path().join("NanaZip.Universal.Console.exe"),
            b"unverified",
        )
        .unwrap();
        assert!(verify_backend_at(tmp.path()).is_err());
    }

    #[test]
    #[cfg(windows)]
    fn restricted_dispatch_uses_the_real_official_backend_and_checks_unselected_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("source.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("chosen.txt", options).unwrap();
        zip.write_all(b"chosen contents").unwrap();
        zip.start_file("other.txt", options).unwrap();
        zip.write_all(b"unique-content-to-corrupt").unwrap();
        zip.finish().unwrap();
        let path_str = path.to_str().unwrap();
        let request = EngineOptions {
            selected: vec!["chosen.txt".into()],
            ..Default::default()
        };
        with_rules([&["zip"], &[], &[], &[]], || {
            let snapshot = scope();
            assert!(snapshot.restricted());
            assert_eq!(
                engine::executable().unwrap(),
                snapshot.official_executable().unwrap()
            );
            let dest = tmp.path().join("selected");
            assert_eq!(
                operations::extract_archive(path_str, dest.to_str().unwrap(), &request).unwrap(),
                1
            );
            assert_eq!(
                std::fs::read(dest.join("chosen.txt")).unwrap(),
                b"chosen contents"
            );
            assert!(!dest.join("other.txt").exists());
            assert_eq!(
                operations::preview_archive(path_str, "chosen.txt", &request).unwrap(),
                b"chosen contents"
            );
            assert_eq!(
                operations::list_archive(path_str, &request).unwrap().len(),
                2
            );
            operations::test_archive(path_str, &request).unwrap();
        });
        let mut bytes = std::fs::read(&path).unwrap();
        let marker = b"unique-content-to-corrupt";
        let position = bytes
            .windows(marker.len())
            .position(|s| s == marker)
            .unwrap();
        bytes[position] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        with_rules([&["zip"], &[], &[], &[]], || {
            let dest = tmp.path().join("corrupt");
            assert!(
                operations::extract_archive(path_str, dest.to_str().unwrap(), &request).is_err()
            );
            assert!(!dest.exists());
        });
    }

    #[test]
    #[cfg(windows)]
    fn allowed_split_copy_uses_full_official_extraction_when_policy_is_active() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let small = b"chosen bytes";
        let other = b"unselected Copy member marker for policy test".repeat(256);
        std::fs::write(root.join("chosen.txt"), small).unwrap();
        std::fs::write(root.join("other.bin"), &other).unwrap();
        let created = std::process::Command::new(engine::executable().unwrap())
            .current_dir(root)
            .args([
                "a",
                "-t7z",
                "-mx=0",
                "-ms=off",
                "source.7z",
                "chosen.txt",
                "other.bin",
            ])
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let raw = std::fs::read(root.join("source.7z")).unwrap();
        let member = raw
            .windows(other.len())
            .position(|b| b == other.as_slice())
            .unwrap();
        let part = |i| root.join(format!("source.7z.{i:03}"));
        for (i, chunk) in raw.chunks(1024).enumerate() {
            std::fs::write(part(i + 1), chunk).unwrap();
        }
        let opts = EngineOptions {
            selected: vec!["chosen.txt".into()],
            ..Default::default()
        };
        with_rules(
            [&["Split", "7z"], &[], &["Copy", "LZMA", "LZMA2"], &[]],
            || {
                let staging = root.join("valid");
                let calls = trace_archive_children(|| {
                    assert_eq!(
                        operations::extract_archive(
                            part(1).to_str().unwrap(),
                            staging.to_str().unwrap(),
                            &opts
                        )
                        .unwrap(),
                        1
                    );
                });
                assert!(calls.iter().any(|(_, op)| op == "x"));
                assert!(calls
                    .iter()
                    .all(|(path, _)| path.file_name().unwrap() == "NanaZip.Universal.Console.exe"));
                assert_eq!(std::fs::read(staging.join("chosen.txt")).unwrap(), small);
                assert!(!staging.join("other.bin").exists());
            },
        );
        let changed = part(member / 1024 + 1);
        let mut chunk = std::fs::read(&changed).unwrap();
        chunk[member % 1024] ^= 1;
        std::fs::write(changed, chunk).unwrap();
        with_rules(
            [&["Split", "7z"], &[], &["Copy", "LZMA", "LZMA2"], &[]],
            || {
                let staging = root.join("corrupt");
                assert!(operations::extract_archive(
                    part(1).to_str().unwrap(),
                    staging.to_str().unwrap(),
                    &opts
                )
                .is_err());
                assert!(!staging.exists());
            },
        );
    }
}

#[cfg(test)]
thread_local! { static CHILDREN: RefCell<Option<Vec<(PathBuf, String)>>> = const { RefCell::new(None) }; }
#[cfg(test)]
pub(crate) fn child_started(program: &std::ffi::OsStr, operation: &str) {
    CHILDREN.with(|slot| {
        if let Some(calls) = slot.borrow_mut().as_mut() {
            calls.push((program.into(), operation.into()));
        }
    });
}
#[cfg(test)]
fn trace_archive_children(run: impl FnOnce()) -> Vec<(PathBuf, String)> {
    CHILDREN.with(|slot| slot.replace(Some(Vec::new())));
    run();
    CHILDREN.with(|slot| slot.borrow_mut().take().unwrap())
}
