use super::*;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::Child,
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    System::Com::{CoCreateGuid, CoCreateInstance},
    UI::Shell::{Common::ITEMIDLIST, SHCreateShellItemArrayFromIDLists, SHParseDisplayName},
};

// Launch a separate test process with a fresh, unregistered class ID. This
// exercises the real Windows marshaler without touching Explorer's settings.
const TEST_DIR: &str = "QING_ZIP_COM_TEST_DIRECTORY";
const TEST_ID: &str = "QING_ZIP_COM_TEST_CLSID";
const TEST_IDLE: &str = "QING_ZIP_COM_TEST_IDLE_MS";
// Optional second hop using qing-shell/tests/argv-fixture.cpp. This fixture
// records the argument vector parsed by the actual Windows child process.
const ARGV_RECORDER: &str = "QING_ZIP_COM_TEST_ARGV_RECORDER";

#[test]
fn private_startup_modes_require_their_exact_arguments() {
    let mode = |a: &[&str]| startup_mode(a.iter().map(std::ffi::OsString::from));
    assert!(mode(&[]).is_none());
    assert!(mode(&["--compress-files", "one"]).is_none());
    for args in [
        &["--shell-compress-server"][..],
        &["--shell-compress-server", "-Embedding"],
        &["--shell-compress-server", "/embedding"],
    ] {
        assert_eq!(mode(args).unwrap().unwrap(), Startup::Server);
    }
    assert_eq!(
        mode(&["--unregister-compress-menu"]).unwrap().unwrap(),
        Startup::Unregister
    );
    for args in [
        &["--unregister-compress-menu", "extra"][..],
        &["--shell-compress-server", "unexpected"],
        &["--shell-compress-server", "-Embedding", "extra"],
    ] {
        assert!(mode(args).unwrap().is_err());
    }
}

fn record(paths: &[String]) -> Result<()> {
    let root = PathBuf::from(std::env::var_os(TEST_DIR).unwrap());
    if let Some(path) = std::env::var_os(ARGV_RECORDER) {
        check_child_argv(Path::new(&path), paths, &root)?;
    }
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls"))?;
    writeln!(log, "{}", serde_json::to_string(paths).unwrap())?;
    Ok(())
}

fn check_child_argv(exe: &Path, paths: &[String], root: &Path) -> Result<()> {
    let output = root.join("argv");
    let status = compression_command(exe, paths)?
        .env("QING_SHELL_TEST_OUTPUT", &output)
        .status()?;
    if !status.success() {
        return Err(Error::new(E_FAIL, "argument recorder failed"));
    }
    let bytes = fs::read(output)?;
    assert_eq!(bytes.len() % 2, 0);
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let args: Vec<_> = units
        .strip_suffix(&[0])
        .unwrap()
        .split(|u| *u == 0)
        .map(|s| String::from_utf16(s).unwrap())
        .collect();
    assert_eq!(Path::new(&args[0]), exe);
    assert_eq!(args[1], SWITCH);
    assert_eq!(&args[2..], paths);
    Ok(())
}

#[test]
fn server_child() {
    let Some(root) = std::env::var_os(TEST_DIR) else {
        return;
    };
    let clsid =
        GUID::from_u128(u128::from_str_radix(&std::env::var(TEST_ID).unwrap(), 16).unwrap());
    let idle = std::env::var(TEST_IDLE).unwrap().parse().unwrap();
    serve(&clsid, record, idle, || {
        fs::write(PathBuf::from(root).join("ready"), b"")?;
        Ok(())
    })
    .unwrap();
}

struct Server(Child);
impl Server {
    fn start(root: &Path, idle: u32) -> (Self, GUID) {
        let id = unsafe { CoCreateGuid().unwrap() };
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shell_compress::tests::server_child",
                "--nocapture",
            ])
            .env(TEST_DIR, root)
            .env(TEST_ID, format!("{:032x}", id.to_u128()))
            .env(TEST_IDLE, idle.to_string())
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let mut server = Self(child);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !root.join("ready").exists() {
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "COM server exited before registration"
            );
            assert!(
                Instant::now() < deadline,
                "COM server registration timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
        (server, id)
    }
    fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "COM server failed: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "COM server did not terminate");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn shell_array(paths: &[PathBuf]) -> IShellItemArray {
    struct Ids(Vec<*const ITEMIDLIST>);
    impl Drop for Ids {
        fn drop(&mut self) {
            for &id in &self.0 {
                unsafe { CoTaskMemFree(Some(id.cast())) }
            }
        }
    }
    let mut ids = Ids(Vec::new());
    for path in paths {
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut id = std::ptr::null_mut();
        unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut id, 0, None).unwrap() };
        ids.0.push(id);
    }
    unsafe { SHCreateShellItemArrayFromIDLists(&ids.0).unwrap() }
}

#[test]
fn explorer_selection_crosses_the_process_boundary_as_one_complete_group() {
    let _com = Apartment::open().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let (mut server, clsid) = Server::start(root, 30000);
    let execute: IExecuteCommand =
        unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER).unwrap() };
    let selection: IObjectWithSelection = execute.cast().unwrap();
    let paths: Vec<_> = ["a b &(1).txt", "中文 'quoted' 🧊.zip"]
        .iter()
        .map(|n| root.join(n))
        .collect();
    for path in &paths {
        fs::write(path, b"test").unwrap();
    }
    let expected: Vec<_> = paths
        .iter()
        .map(|p| crate::instance::canonical_archive_path(p.to_str().unwrap().into()).unwrap())
        .collect();
    unsafe {
        assert!(execute.Execute().is_err());
        selection
            .SetSelection(&shell_array(&[paths[0].clone(), root.into()]))
            .unwrap();
        assert!(execute.Execute().is_err());
        assert!(!root.join("calls").exists());
        selection.SetSelection(&shell_array(&paths)).unwrap();
        assert_eq!(
            selection
                .GetSelection::<IShellItemArray>()
                .unwrap()
                .GetCount()
                .unwrap(),
            2
        );
        // The paths come from the selection. Shell-supplied parameters and
        // working-directory hints cannot inject additional arguments.
        execute
            .SetParameters(windows::core::w!("--unregister-compress-menu"))
            .unwrap();
        execute.SetDirectory(windows::core::w!("C:\\")).unwrap();
        execute.Execute().unwrap();
        assert!(execute.Execute().is_err());
        selection.SetSelection(None).unwrap();
        assert!(execute.Execute().is_err());
    }
    let log = fs::read_to_string(root.join("calls")).unwrap();
    let calls: Vec<Vec<String>> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls, [expected]);
    drop(selection);
    drop(execute);
    server.wait_for_exit();
}

#[test]
fn unactivated_server_exits_and_long_commands_do_not_split() {
    let temp = tempfile::tempdir().unwrap();
    let (mut server, _) = Server::start(temp.path(), 300);
    server.wait_for_exit();
    assert!(!temp.path().join("calls").exists());
    let exe = Path::new(r"C:\Apps\qing-zip.exe");
    let values = vec!["a b".to_owned(), "字'\" 🧊".to_owned()];
    let cmd = compression_command(exe, &values).unwrap();
    assert_eq!(cmd.get_program(), exe);
    assert_eq!(
        cmd.get_args().collect::<Vec<_>>(),
        [SWITCH, &values[0], &values[1]]
    );
    assert!(compression_command(exe, &vec!["🧊".repeat(256); 128]).is_err());
    if let Some(fixture) = std::env::var_os(ARGV_RECORDER) {
        check_child_argv(Path::new(&fixture), &values, temp.path()).unwrap();
    }
}

#[test]
fn complete_selection_limit_and_reparse_ancestors_are_enforced() {
    use std::os::windows::process::CommandExt;
    let _com = Apartment::open().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    let files: Vec<_> = (0..129)
        .map(|i| {
            let path = target.join(format!("{i:03}.txt"));
            fs::write(&path, b"x").unwrap();
            path
        })
        .collect();
    assert_eq!(
        selected_paths(&shell_array(&files[..128])).unwrap().len(),
        128
    );
    assert!(selected_paths(&shell_array(&files)).is_err());
    let junction = Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J", "junction", "target"])
        .current_dir(root)
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(junction.status.success(), "could not create test junction");
    let linked = root.join("junction").join("000.txt");
    assert!(selected_paths(&shell_array(&[files[0].clone(), linked])).is_err());
}
