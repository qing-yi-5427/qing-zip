//! Explicit file-workspace projection, prepared while the target edit lock is held.
use super::*;
use std::io::Write;
pub(super) mod move_proof;

pub(super) struct Prepared {
    pub names: Vec<String>,
    pub cwd: PathBuf,
    _directory: ShellDragDirectory,
    pub proof: Option<move_proof::Plan>,
}

fn key(name: &str) -> String {
    name.replace('\\', "/").trim_end_matches('/').to_lowercase()
}
fn member(name: &str) -> Result<String, String> {
    let name = name.replace('\\', "/");
    let name = name.trim_end_matches('/');
    validate_name(name)?;
    Ok(name.to_owned())
}

fn inventory(entries: Vec<Entry>) -> Result<BTreeMap<String, bool>, String> {
    let mut result = BTreeMap::new();
    let mut explicit = HashSet::new();
    for entry in entries {
        let name = member(&entry.name)?;
        let k = key(&name);
        if !explicit.insert(k.clone()) {
            return Err("目标压缩包存在重复或大小写冲突的成员".into());
        }
        if result
            .insert(k, entry.is_dir)
            .is_some_and(|old| old != entry.is_dir)
        {
            return Err("目标压缩包存在文件与目录冲突".into());
        }
        let mut parent = name.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if result.insert(key(prefix), true) == Some(false) {
                return Err("目标压缩包存在文件与目录冲突".into());
            }
            parent = prefix;
        }
    }
    Ok(result)
}

// Directories merge; conflicts on files obey the explicit workspace policy.
// Type collisions never replace a whole target subtree implicitly.
fn plan(
    directory: &Path,
    relative: &str,
    occupied: &mut BTreeMap<String, bool>,
    policy: &str,
    names: &mut Vec<String>,
    ctl: &JobControl,
    proof: &mut Option<move_proof::Plan>,
) -> Result<(), String> {
    ctl.check()?;
    let mut relative = member(relative)?;
    let mut path = directory.join(&relative);
    no_links(&path)?;
    let is_dir = path.is_dir();
    if let Some(existing_dir) = occupied.get(&key(&relative)).copied() {
        if !is_dir || !existing_dir {
            match policy {
                "skip" => { if let Some(p)=proof { p.skip(&relative, "目标已存在，按设置跳过"); } return Ok(()); },
                "rename" => {
                    let original = relative.clone();
                    let (parent, leaf) =
                        original.rsplit_once('/').unwrap_or(("", original.as_str()));
                    let leaf_path = Path::new(leaf);
                    let stem = if is_dir {
                        leaf
                    } else {
                        leaf_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .ok_or("invalid input name")?
                    };
                    let suffix = if is_dir {
                        String::new()
                    } else {
                        leaf_path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|v| format!(".{v}"))
                            .unwrap_or_default()
                    };
                    let mut found = None;
                    for n in 2..100_002 {
                        ctl.check()?;
                        let leaf = format!("{stem} ({n}){suffix}");
                        let candidate = if parent.is_empty() {
                            leaf
                        } else {
                            format!("{parent}/{leaf}")
                        };
                        if !occupied.contains_key(&key(&candidate))
                            && !directory.join(&candidate).exists()
                        {
                            found = Some(candidate);
                            break;
                        }
                    }
                    relative = found.ok_or("无法找到可用名称")?;
                    if let Some(p)=proof { p.rename(&original, &relative); }
                    let renamed = directory.join(&relative);
                    fs::rename(&path, &renamed).map_err(|e| e.to_string())?;
                    path = renamed;
                }
                "overwrite" if !is_dir && !existing_dir => (),
                "overwrite" => return Err(format!("文件与目录类型冲突，未替换：{relative}")),
                _ => return Err(format!("目标已存在：{relative}")),
            }
        }
    }
    occupied.insert(key(&relative), is_dir);
    if is_dir {
        let mut children = fs::read_dir(&path)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        children.sort();
        if children.is_empty() {
            names.push(relative);
        } else {
            for child in children {
                let child = child.to_str().ok_or("invalid input filename")?;
                plan(
                    directory,
                    &format!("{relative}/{child}"),
                    occupied,
                    policy,
                    names,
                    ctl,
                    proof,
                )?;
            }
        }
    } else {
        names.push(relative);
    }
    Ok(())
}

// Snapshot the readable primary stream, as ordinary archive add does. NTFS
// storage representation (sparse/compressed/EFS) is not a copy requirement.
fn snapshot(
    source: &Path,
    target: &Path,
    ctl: &JobControl,
    parents: &mut CheckedDirectories,
    proof: &mut Option<move_proof::Plan>,
    snapshot_root: &Path,
    opts: &EngineOptions,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    ctl.check()?;
    parents.check(source.parent().ok_or("missing source parent")?)?;
    let opened=crate::source_access::open(source,crate::source_access::OpenSpec{access:0x80000000,share:1,flags:0x02200000},opts.preserve_input_atime).map_err(|e|e.to_string())?;
    let status=opened.preservation;
    let mut input=opened.file;
    let meta = crate::private_stage::checked_metadata(&input)?;
    if meta.file_attributes() & 0x400 != 0 || (!meta.is_file() && !meta.is_dir()) {
        return Err("不能添加链接或特殊对象".into());
    }
    if meta.is_file(){if let Some(report)=&opts.input_atime_report{report.lock().unwrap_or_else(|e|e.into_inner()).record(source,status);}}
    let mut primary_sha = None;
    let output = if meta.is_dir() {
        fs::create_dir(target).map_err(|e| e.to_string())?;
        for child in fs::read_dir(source).map_err(|e| e.to_string())? {
            ctl.check()?;
            let child = child.map_err(|e| e.to_string())?;
            snapshot(&child.path(), &target.join(child.file_name()), ctl, parents, proof, snapshot_root, opts)?;
        }
        File::options()
            .access_mode(0xc0000100)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(target)
            .map_err(|e| e.to_string())?
    } else {
        let mut output = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut digest = Sha256::new();
        let mut length = 0u64;
        #[cfg(test)] BEFORE_SOURCE_READ.with(|hook|if let Some(check)=hook.borrow_mut().as_mut(){check(source,&mut input);});
        loop {
            ctl.check()?;
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            digest.update(&buffer[..n]);
            length += n as u64;
        }
        if length != meta.len() {
            return Err("源文件大小发生变化".into());
        }
        output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut verified = Sha256::new();
        loop {
            ctl.check()?;
            let n = output.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            verified.update(&buffer[..n]);
        }
        let expected: [u8;32] = digest.finalize().into();
        if expected != <[u8;32]>::from(verified.finalize()) {
            return Err("临时输入完整性验证失败".into());
        }
        primary_sha=Some(expected);
        output
    };
    // Recreate inexpensive storage markers when the private filesystem supports
    // them, so native archive attributes match ordinary add. Unsupported storage
    // representation never prevents archiving an otherwise readable stream.
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            file: *mut std::ffi::c_void,
            code: u32,
            input: *const u8,
            input_len: u32,
            output: *mut u8,
            output_len: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let mut returned = 0;
    if meta.file_attributes() & 0x200 != 0 && meta.is_file() {
        unsafe {
            DeviceIoControl(
                output.as_raw_handle(),
                0x900c4,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
    }
    if meta.file_attributes() & 0x800 != 0 {
        let compression: u16 = 1;
        unsafe {
            DeviceIoControl(
                output.as_raw_handle(),
                0x9c040,
                (&compression as *const u16).cast(),
                2,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            );
        }
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            class: i32,
            data: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    // Preserve archive-relevant basic timestamps and attributes, not source ACL
    // or storage-only flags. The private output remains an ordinary stream.
    let attrs = meta.file_attributes() & 0x3127;
    let basic = [
        meta.creation_time(),
        meta.last_access_time(),
        meta.last_write_time(),
        0,
        u64::from(if attrs == 0 { 0x80 } else { attrs }),
    ];
    if unsafe { SetFileInformationByHandle(output.as_raw_handle(), 0, basic.as_ptr().cast(), 40) }
        == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if let Some(p)=proof {
        p.capture(source, target.strip_prefix(snapshot_root).map_err(|e|e.to_string())?, &input, meta.is_dir(), meta.len(), primary_sha)?;
    }
    ctl.check()
}

pub(super) fn prepare(
    archive: &Path,
    inputs: &[String],
    folder: &str,
    opts: &EngineOptions,
) -> Result<Prepared, String> {
    prepare_internal(archive, inputs, folder, opts, None, false)
}

pub(super) fn prepare_compression(archive: &Path, inputs: &[String], folder: &str, opts: &EngineOptions) -> Result<Prepared, String> {
    prepare_internal(archive, inputs, folder, opts, None, true)
}

#[allow(dead_code)]
pub(super) fn prepare_for_move_proof(archive: &Path, inputs: &[String], folder: &str, opts: &EngineOptions) -> Result<Prepared,String> {
    prepare_internal(archive, inputs, folder, opts, Some(move_proof::Plan::default()), false)
}
fn prepare_internal(archive: &Path, inputs: &[String], folder: &str, opts: &EngineOptions, mut proof: Option<move_proof::Plan>, compression: bool) -> Result<Prepared,String> {
    let ctl = control(opts);
    ctl.check()?;
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("无效的冲突处理方式".into());
    }
    let folder = if folder.is_empty() {
        String::new()
    } else {
        member(folder)?
    };
    let paths = input_paths(inputs)?;
    reject_output_in_inputs(archive.parent().ok_or("missing archive parent")?, &paths)?;
    let canonical = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    if paths
        .iter()
        .any(|p| fs::canonicalize(p).ok().as_ref() == Some(&canonical))
    {
        return Err("cannot add an archive to itself".into());
    }
    let mut occupied = target_inventory(archive, &folder, opts)?;
    let directory = ShellDragDirectory::create_named("qing-zip-archive-add-")?;
    let dest = directory.path().join(&folder);
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let mut parents = CheckedDirectories::default();
    for path in &paths {
        snapshot(
            path,
            &dest.join(path.file_name().ok_or("missing input name")?),
            &ctl,
            &mut parents,
            &mut proof,
            directory.path(),
            opts,
        )?;
    }
    let cwd = directory.path().to_owned();
    if compression {
        let mut names = Vec::with_capacity(paths.len());
        for path in &paths {
            let leaf = path.file_name().and_then(|name| name.to_str()).ok_or("invalid input filename")?;
            let relative = format!("{folder}/{leaf}");
            // Keep directory roots in the engine selection. Sync may then remove
            // absent descendants of these roots while keeping other members.
            plan(&cwd, &relative, &mut occupied, "overwrite", &mut Vec::new(), &ctl, &mut proof)?;
            names.push(relative);
        }
        return Ok(Prepared { names, cwd, _directory: directory, proof });
    }
    finish_projection(directory, cwd, &folder, occupied, opts, proof)
}

fn target_inventory(
    archive: &Path,
    folder: &str,
    opts: &EngineOptions,
) -> Result<BTreeMap<String, bool>, String> {
    let mut listing_options = opts.clone();
    listing_options.selected.clear();
    let occupied = inventory(list(
        archive.to_str().ok_or("invalid archive path")?,
        &listing_options,
    )?)?;
    let mut prefix = String::new();
    for component in folder.split('/').filter(|s| !s.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
        if occupied.get(&key(&prefix)) == Some(&false) {
            return Err(format!("目标目录被文件占用：{prefix}"));
        }
    }
    Ok(occupied)
}

pub(super) fn prepare_export(
    archive: &Path,
    export: archive_export::Export,
    folder: &str,
    opts: &EngineOptions,
) -> Result<Prepared, String> {
    let ctl = control(opts);
    ctl.check()?;
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("无效的冲突处理方式".into());
    }
    let folder = if folder.is_empty() {
        String::new()
    } else {
        member(folder)?
    };
    let occupied = target_inventory(archive, &folder, opts)?;
    let (directory, roots) = export.into_parts();
    let cwd = directory.path().join("import");
    let dest = cwd.join(&folder);
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    for root in roots {
        ctl.check()?;
        let root = Path::new(&root);
        no_links(root)?;
        // These roots are exclusively owned by the verified Export. Moving
        // inside its same private root retains object identity and metadata;
        // there is no second payload copy or source-archive mutation.
        if !root.starts_with(directory.path().join("data")) {
            return Err("invalid private export provenance".into());
        }
        let target = dest.join(root.file_name().ok_or("invalid export root")?);
        if target.try_exists().map_err(|e| e.to_string())? {
            return Err("导出项目重名".into());
        }
        fs::rename(root, &target).map_err(|e| e.to_string())?;
    }
    finish_projection(directory, cwd, &folder, occupied, opts, None)
}

fn finish_projection(
    directory: ShellDragDirectory,
    cwd: PathBuf,
    folder: &str,
    mut occupied: BTreeMap<String, bool>,
    opts: &EngineOptions,
    mut proof: Option<move_proof::Plan>,
) -> Result<Prepared, String> {
    let ctl = control(opts);
    let dest = cwd.join(folder);
    let mut roots = fs::read_dir(&dest)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    roots.sort();
    let mut names = Vec::new();
    for root in roots {
        let root = root.to_str().ok_or("invalid input filename")?;
        let relative = if folder.is_empty() {
            root.to_owned()
        } else {
            format!("{folder}/{root}")
        };
        plan(
            &cwd,
            &relative,
            &mut occupied,
            &opts.overwrite,
            &mut names,
            &ctl,
            &mut proof,
        )?;
    }
    #[cfg(test)]
    AFTER_SNAPSHOT.with(|hook| {
        if let Some(run) = hook.borrow_mut().take() {
            run(directory.path());
        }
    });
    ctl.check()?;
    if let Some(p)=&proof { p.validate_mapping()?; }
    Ok(Prepared {
        names,
        cwd,
        _directory: directory,
        proof,
    })
}

#[cfg(test)]
thread_local! {
    static BEFORE_SOURCE_READ:std::cell::RefCell<Option<Box<dyn FnMut(&Path,&mut File)>>>=const{std::cell::RefCell::new(None)};
    static AFTER_SNAPSHOT: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_input_atime_snapshot_hooks_cover_each_primary_read_and_receipt(){
        use std::os::windows::io::AsRawHandle;
        let(_temp,archive,local)=fixture("zip");fs::create_dir_all(local.join("deep")).unwrap();fs::write(local.join("one.txt"),b"one").unwrap();fs::write(local.join("deep/two.txt"),b"two").unwrap();fs::write(local.join("empty.txt"),b"").unwrap();
        let seen=std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));let capture=seen.clone();
        crate::source_access::SUPPRESS_CALLS.with(|v|v.borrow_mut().clear());
        BEFORE_SOURCE_READ.with(|h|*h.borrow_mut()=Some(Box::new(move|path,file|{assert_eq!(file.stream_position().unwrap(),0);assert!(crate::source_access::SUPPRESS_CALLS.with(|v|v.borrow_mut().pop())==Some(file.as_raw_handle() as usize),"first read must follow suppression on this exact handle");capture.borrow_mut().push(path.to_owned());})));
        let response=crate::operations::execute(crate::operations::Request{operation:"archive-add".into(),path:archive.display().to_string(),dest:"projected".into(),inputs:vec![local.display().to_string()],options:EngineOptions{preserve_input_atime:true,..Default::default()},..Default::default()}).unwrap();
        BEFORE_SOURCE_READ.with(|h|h.borrow_mut().take());
        assert_eq!(response["input_atime"]["applied_handles"].as_u64(),Some(seen.borrow().len() as u64));assert!(seen.borrow().len()>=3);assert_eq!(response["input_atime"]["unavailable_handles"],0);assert_eq!(response["input_atime"]["backend_best_effort"],false);assert_eq!(response["input_atime"]["directory_enumeration_covered"],false);
        assert_eq!(fs::read(local.join("one.txt")).unwrap(),b"one");test(archive.to_str().unwrap(),&EngineOptions::default()).unwrap();
    }
    #[test]
    fn preserve_input_atime_add_failures_reported_and_ordinary_backend_unconfirmed(){
        for ordinary in [false,true]{let(_temp,archive,local)=fixture("zip");let input=local.join("new.txt");fs::write(&input,b"payload").unwrap();
            if !ordinary{crate::source_access::FAIL_SUPPRESS.with(|v|v.set(true));}
            let response=crate::operations::execute(crate::operations::Request{operation:if ordinary{"add"}else{"archive-add"}.into(),path:archive.display().to_string(),inputs:vec![input.display().to_string()],options:EngineOptions{preserve_input_atime:true,..Default::default()},..Default::default()}).unwrap();
            assert_eq!(response["input_atime"]["backend_best_effort"],ordinary);
            if !ordinary{assert_eq!(response["input_atime"]["unavailable_handles"],1);assert_eq!(response["input_atime"]["warnings"][0]["phase"],"suppress_updates");assert_eq!(response["input_atime"]["warnings"][0]["os_error"],50);}
            assert_eq!(fs::read(&input).unwrap(),b"payload");test(archive.to_str().unwrap(),&EngineOptions::default()).unwrap();
        }
        let disabled:EngineOptions=serde_json::from_str("{}").unwrap();assert!(!disabled.preserve_input_atime);
    }
    fn fixture(format: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let seed = temp.path().join("seed");
        fs::create_dir_all(seed.join("folder/deep")).unwrap();
        fs::write(seed.join("folder/a.txt"), b"old").unwrap();
        fs::write(seed.join("folder/deep/kept.txt"), b"unrelated").unwrap();
        let archive = temp.path().join(format!("archive.{format}"));
        create(
            archive.to_str().unwrap(),
            &[seed.join("folder").display().to_string()],
            format,
            &EngineOptions::default(),
        )
        .unwrap();
        let local = temp.path().join("local");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("a.txt"), b"new").unwrap();
        fs::write(local.join("b.txt"), b"added").unwrap();
        (temp, archive, local)
    }
    fn contents(archive: &Path, output: &Path) -> BTreeMap<String, Vec<u8>> {
        extract(
            archive.to_str().unwrap(),
            output.to_str().unwrap(),
            &EngineOptions::default(),
        )
        .unwrap();
        fn walk(root: &Path, path: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
            for item in fs::read_dir(path).unwrap() {
                let p = item.unwrap().path();
                if p.is_dir() {
                    walk(root, &p, result);
                } else {
                    result.insert(
                        p.strip_prefix(root)
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .replace('\\', "/"),
                        fs::read(p).unwrap(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        walk(output, output, &mut result);
        result
    }

    #[test]
    fn workspace_archive_add_four_policies_preserve_subfolder_and_unrelated_members() {
        for format in ["zip", "7z"] {
            for policy in ["error", "skip", "rename", "overwrite"] {
                let (temp, archive, local) = fixture(format);
                let original = fs::read(&archive).unwrap();
                let opts = EngineOptions {
                    overwrite: policy.into(),
                    ..Default::default()
                };
                let result = crate::operations::execute_request(crate::operations::Request {
                    operation: "archive-add".into(),
                    path: archive.display().to_string(),
                    dest: "folder/".into(),
                    inputs: vec![
                        local.join("a.txt").display().to_string(),
                        local.join("b.txt").display().to_string(),
                    ],
                    options: opts,
                    ..Default::default()
                });
                if policy == "error" {
                    assert!(result.is_err());
                    assert_eq!(fs::read(&archive).unwrap(), original);
                    continue;
                }
                result.unwrap();
                let files = contents(&archive, &temp.path().join("out"));
                assert_eq!(
                    files.get("folder/a.txt").unwrap(),
                    if policy == "overwrite" {
                        b"new"
                    } else {
                        b"old"
                    }
                );
                assert_eq!(files.get("folder/b.txt").unwrap(), b"added");
                assert_eq!(files.get("folder/deep/kept.txt").unwrap(), b"unrelated");
                assert!(!files.contains_key("a.txt"));
                assert_eq!(
                    files.get("folder/a (2).txt").map(Vec::as_slice),
                    if policy == "rename" {
                        Some(b"new".as_slice())
                    } else {
                        None
                    }
                );
            }
        }
    }

    #[test]
    fn workspace_archive_add_cancel_and_corrupt_proposal_preserve_original_bytes() {
        for cancel in [false, true] {
            let (_temp, archive, local) = fixture("zip");
            let original = fs::read(&archive).unwrap();
            let ctl = JobControl::new();
            let c = ctl.clone();
            update_modes_tests::AFTER_ENGINE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |staged| {
                    if cancel {
                        c.cancel();
                    } else {
                        fs::write(staged, b"broken").unwrap();
                    }
                }))
            });
            let opts = EngineOptions {
                overwrite: "overwrite".into(),
                control: Some(ctl),
                ..Default::default()
            };
            assert!(archive_add(
                archive.to_str().unwrap(),
                &[local.join("a.txt").display().to_string()],
                "folder",
                &opts
            )
            .is_err());
            assert_eq!(fs::read(&archive).unwrap(), original);
        }
    }

    #[test]
    fn workspace_archive_add_skip_all_is_byte_identical_and_rejects_type_collision() {
        let (_temp, archive, local) = fixture("zip");
        let original = fs::read(&archive).unwrap();
        archive_add(
            archive.to_str().unwrap(),
            &[local.join("a.txt").display().to_string()],
            "folder",
            &EngineOptions {
                overwrite: "skip".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fs::read(&archive).unwrap(), original);
        for folder in ["folder/a.txt", "../outside", "/absolute"] {
            assert!(archive_add(
                archive.to_str().unwrap(),
                &[local.join("b.txt").display().to_string()],
                folder,
                &EngineOptions::default()
            )
            .is_err());
            assert_eq!(fs::read(&archive).unwrap(), original);
        }
    }
    #[test]
    fn workspace_archive_add_directory_projection_and_skipped_members_are_exact() {
        let (temp, archive, local) = fixture("zip");
        fs::create_dir_all(local.join("folder/empty")).unwrap();
        fs::create_dir_all(local.join("folder/deep")).unwrap();
        fs::write(local.join("folder/a.txt"), b"must-skip").unwrap();
        fs::write(local.join("folder/deep/new.txt"), b"new-child").unwrap();
        archive_add(
            archive.to_str().unwrap(),
            &[local.join("folder").display().to_string()],
            "",
            &EngineOptions {
                overwrite: "skip".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let files = contents(&archive, &temp.path().join("out"));
        assert_eq!(files.get("folder/a.txt").unwrap(), b"old");
        assert_eq!(files.get("folder/deep/new.txt").unwrap(), b"new-child");
        assert_eq!(files.get("folder/deep/kept.txt").unwrap(), b"unrelated");
        assert!(temp.path().join("out/folder/empty").is_dir());
    }

    #[test]
    fn workspace_archive_add_cancel_after_snapshot_cleans_private_inputs() {
        let (_temp, archive, local) = fixture("zip");
        let original = fs::read(&archive).unwrap();
        let stage = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = stage.clone();
        let ctl = JobControl::new();
        let c = ctl.clone();
        AFTER_SNAPSHOT.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                *saved.borrow_mut() = Some(path.to_owned());
                c.cancel();
            }))
        });
        assert!(archive_add(
            archive.to_str().unwrap(),
            &[local.join("b.txt").display().to_string()],
            "folder",
            &EngineOptions {
                control: Some(ctl),
                ..Default::default()
            }
        )
        .is_err());
        assert_eq!(fs::read(&archive).unwrap(), original);
        assert!(!stage.borrow().as_ref().unwrap().exists());
    }

    #[test]
    fn workspace_archive_add_rejects_unsafe_existing_names_before_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("unsafe.zip");
        let input = temp.path().join("safe.txt");
        fs::write(&input, b"safe").unwrap();
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"unsafe").unwrap();
        writer.finish().unwrap();
        let before = fs::read(&archive).unwrap();
        assert!(archive_add(
            archive.to_str().unwrap(),
            &[input.display().to_string()],
            "",
            &EngineOptions::default()
        )
        .is_err());
        assert_eq!(fs::read(&archive).unwrap(), before);
    }

    #[test]
    fn workspace_archive_add_sparse_and_compressed_sources_keep_primary_data_and_times() {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        extern "system" {
            fn DeviceIoControl(
                file: *mut std::ffi::c_void,
                code: u32,
                input: *const u8,
                input_len: u32,
                output: *mut u8,
                output_len: u32,
                returned: *mut u32,
                overlapped: *mut std::ffi::c_void,
            ) -> i32;
        }
        for code in [0x900c4, 0x9c040] {
            let (temp, archive, local) = fixture("zip");
            let path = local.join("special.txt");
            let bytes = vec![42u8; 128 * 1024];
            fs::write(&path, &bytes).unwrap();
            let input = File::options().read(true).write(true).open(&path).unwrap();
            let compression: u16 = 1;
            let mut returned = 0;
            let (ptr, length) = if code == 0x9c040 {
                ((&compression as *const u16).cast(), 2)
            } else {
                (std::ptr::null(), 0)
            };
            assert_ne!(
                unsafe {
                    DeviceIoControl(
                        input.as_raw_handle(),
                        code,
                        ptr,
                        length,
                        std::ptr::null_mut(),
                        0,
                        &mut returned,
                        std::ptr::null_mut(),
                    )
                },
                0,
                "{}",
                std::io::Error::last_os_error()
            );
            input
                .set_times(fs::FileTimes::new().set_modified(
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_650_000_000),
                ))
                .unwrap();
            drop(input);
            let baseline = temp.path().join("baseline.zip");
            create(
                baseline.to_str().unwrap(),
                &[path.display().to_string()],
                "zip",
                &EngineOptions::default(),
            )
            .unwrap();
            archive_add(
                archive.to_str().unwrap(),
                &[path.display().to_string()],
                "folder",
                &EngineOptions::default(),
            )
            .unwrap();
            let files = contents(&archive, &temp.path().join("out"));
            assert_eq!(files.get("folder/special.txt").unwrap(), &bytes);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            let old = list(baseline.to_str().unwrap(), &EngineOptions::default())
                .unwrap()
                .remove(0);
            let new = list(archive.to_str().unwrap(), &EngineOptions::default())
                .unwrap()
                .into_iter()
                .find(|e| e.name.replace('\\', "/") == "folder/special.txt")
                .unwrap();
            assert_eq!(new.modified, old.modified);
            assert_eq!(new.attributes, old.attributes, "storage={code:x}");
        }
    }

    #[test]
    fn workspace_archive_add_readonly_rename_cleans_snapshot_and_duplicate_roots_abort() {
        let (temp, archive, local) = fixture("zip");
        let input = local.join("a.txt");
        let mut permissions = fs::metadata(&input).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&input, permissions).unwrap();
        let stage = std::rc::Rc::new(std::cell::RefCell::new(None));
        let saved = stage.clone();
        AFTER_SNAPSHOT.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                *saved.borrow_mut() = Some(path.to_owned())
            }))
        });
        archive_add(
            archive.to_str().unwrap(),
            &[input.display().to_string()],
            "folder",
            &EngineOptions {
                overwrite: "rename".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !stage.borrow().as_ref().unwrap().exists(),
            "private readonly snapshot leaked"
        );
        let files = contents(&archive, &temp.path().join("out"));
        assert_eq!(files.get("folder/a.txt").unwrap(), b"old");
        assert_eq!(files.get("folder/a (2).txt").unwrap(), b"new");
        assert!(fs::metadata(&input).unwrap().permissions().readonly());
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("a.txt"), b"other").unwrap();
        let before = fs::read(&archive).unwrap();
        assert!(archive_add(
            archive.to_str().unwrap(),
            &[
                input.display().to_string(),
                other.join("a.txt").display().to_string()
            ],
            "folder",
            &EngineOptions {
                overwrite: "overwrite".into(),
                ..Default::default()
            }
        )
        .is_err());
        assert_eq!(fs::read(&archive).unwrap(), before);
        let mut permissions = fs::metadata(&input).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&input, permissions).unwrap();
    }
}
