//! Retain directory namespace pins until the complete recursive result validates.
use std::{fs::{self, File}, path::{Path, PathBuf}, time::SystemTime};
use super::EngineOptions;
const METADATA_BUDGET: usize = 96 * 1024 * 1024;
const BUDGET_ERROR: &str = "校验目录的元数据超过 96 MiB 预算，请分批选择较小的子目录";
const MAX_DEPTH: usize = 256;
fn reserve(used: &mut usize, bytes: usize) -> Result<(), String> {
    *used = used.saturating_add(bytes);
    if *used > METADATA_BUDGET { return Err(BUDGET_ERROR.into()); }
    Ok(())
}
#[derive(Debug, PartialEq, Eq)]
struct Entry { name: std::ffi::OsString, directory: bool, size: u64, modified: Option<SystemTime> }
struct Directory { path: PathBuf, _file: File, entries: Vec<Entry> }
pub(super) struct Inputs {
    pub files: Vec<String>, pub has_directories: bool,
    directories: Vec<Directory>, _ancestors: Vec<File>, _leaves: Vec<File>,
    seen_files: std::collections::HashSet<PathBuf>, seen_dirs: std::collections::HashSet<PathBuf>, metadata_bytes: usize,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_checksums_keep_paths_and_actual_file_hashes() {
        let tmp=tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sub/empty")).unwrap();
        fs::write(tmp.path().join("same.txt"),b"abc").unwrap();
        fs::write(tmp.path().join("sub/same.txt"),b"").unwrap();
        let path=tmp.path().to_str().unwrap().to_owned();
        for names in [vec![],vec!["SHA256".into(),"CRC32".into()]] {
            let result=super::super::checksum_selected(&path,&names,&Default::default()).unwrap();
            let rows=result["checksums"].as_array().unwrap(); assert_eq!(rows.len(),2);
            assert_ne!(rows[0]["path"],rows[1]["path"]);
            let key=if names.is_empty(){&rows[0]["sha256"]}else{&rows[0]["hashes"]["SHA256"]};
            assert_eq!(key,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        }
        let empty=tmp.path().join("sub/empty");
        assert_eq!(super::super::checksum_selected(empty.to_str().unwrap(),&[],&Default::default()).unwrap()["checksums"],serde_json::json!([]));
    }
    #[test]
    fn recursive_inventory_rejects_change_and_cancel() {
        let tmp=tempfile::tempdir().unwrap();fs::write(tmp.path().join("a"),b"one").unwrap();
        let paths=vec![tmp.path().to_str().unwrap().to_owned()];
        let inventory=Inputs::collect(&paths,&Default::default()).unwrap();
        fs::write(tmp.path().join("new"),b"two").unwrap();
        assert!(inventory.verify(&Default::default()).is_err());
        let ctl=super::super::JobControl::default();ctl.cancel();
        let opts=EngineOptions{control:Some(ctl),..Default::default()};
        assert!(Inputs::collect(&paths,&opts).is_err());
    }
    #[test]
    fn overlapping_directories_and_files_are_hashed_once_but_hardlink_names_remain() {
        let tmp=tempfile::tempdir().unwrap();let sub=tmp.path().join("sub");fs::create_dir(&sub).unwrap();
        let file=sub.join("a");fs::write(&file,b"one").unwrap();let other=sub.join("b");fs::hard_link(&file,&other).unwrap();
        let paths=vec![file.to_str().unwrap().into(),tmp.path().to_str().unwrap().into(),sub.to_str().unwrap().into(),file.to_str().unwrap().into()];
        let inputs=Inputs::collect(&paths,&Default::default()).unwrap();
        assert_eq!(inputs.files.len(),2);assert_eq!(inputs.directories.len(),2);
        assert_eq!(inputs.seen_files.len() + inputs.seen_dirs.len(),4);
    }
    #[cfg(windows)]
    #[test]
    fn enumerated_leaf_cannot_be_replaced_or_rewritten_with_matching_metadata() {
        let tmp=tempfile::tempdir().unwrap();let file=tmp.path().join("a");fs::write(&file,b"abc").unwrap();
        let stamp=fs::metadata(&file).unwrap().modified().unwrap();
        let inputs=Inputs::collect(&[tmp.path().to_str().unwrap().into()],&Default::default()).unwrap();
        assert!(File::options().write(true).open(&file).is_err());
        assert!(fs::rename(&file,tmp.path().join("old")).is_err());
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(),stamp);
        assert_eq!(super::super::checksum_selected(file.to_str().unwrap(),&["SHA256".into()],&Default::default()).unwrap()["hashes"]["SHA256"],"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        inputs.verify(&Default::default()).unwrap();
        drop(inputs);fs::write(&file,b"xyz").unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn six_thousand_files_have_bounded_retained_handles() {
        // Process-wide resource accounting must not include concurrent tests.
        const CHILD: &str = "QING_CHECKSUM_HANDLE_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "checksum::tree::tests::six_thousand_files_have_bounded_retained_handles", "--nocapture", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(output.status.success(), "isolated handle test failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            return;
        }
        #[link(name="kernel32")] extern "system" {
            fn GetCurrentProcess()->*mut std::ffi::c_void;
            fn GetProcessHandleCount(process:*mut std::ffi::c_void,count:*mut u32)->i32;
        }
        let handles=||{let mut n=0;assert_ne!(unsafe{GetProcessHandleCount(GetCurrentProcess(),&mut n)},0);n};
        let tmp=tempfile::tempdir().unwrap();
        for i in 0..6000 { fs::write(tmp.path().join(format!("{i:04}.txt")),b"abc").unwrap(); }
        let baseline=handles();let started=std::time::Instant::now();
        let inputs=Inputs::collect(&[tmp.path().to_str().unwrap().into()],&Default::default()).unwrap();
        let retained=handles();assert_eq!(inputs.seen_files.len() + inputs.seen_dirs.len(),6001);
        assert_eq!(inputs._leaves.len(), 6000);
        assert_eq!(inputs.directories.len(), 1);
        assert!(inputs._ancestors.len() < 100);
        assert!(retained>=baseline+6000);assert!(retained<baseline+6100);
        eprintln!("RECURSIVE_CHECKSUM_DIAGNOSTIC debug=true count=6000 collection_ms={} baseline_handles={baseline} retained_handles={retained} sampled_not_kernel_peak=true",started.elapsed().as_millis());
        drop(inputs);assert!(handles()<baseline+20);
    }
    #[cfg(windows)]
    #[test]
    fn recursive_namespace_pin_and_reparse_rejection() {
        let tmp=tempfile::tempdir().unwrap();let root=tmp.path().join("tree");fs::create_dir(&root).unwrap();
        let paths=vec![root.to_str().unwrap().to_owned()];let inventory=Inputs::collect(&paths,&Default::default()).unwrap();
        assert!(fs::rename(&root,tmp.path().join("moved")).is_err());drop(inventory);
        let target=tmp.path().join("target");fs::create_dir(&target).unwrap();
        let link=root.join("junction");
        let result=std::process::Command::new("cmd").args(["/c","mklink","/J"]).arg(&link).arg(&target).output().unwrap();
        assert!(result.status.success());
        assert!(Inputs::collect(&paths,&Default::default()).is_err());
        fs::remove_dir(link).unwrap();
    }
}
fn regular(meta: &fs::Metadata) -> Result<(), String> {
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return Err("linked checksum input rejected".into()); }
    }
    if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
        return Err("checksums require regular files or directories without links".into());
    }
    Ok(())
}
pub(super) fn is_directory(path: &str) -> Result<bool, String> {
    if path.is_empty() || path.contains(['\r','\n','\0']) { return Err("invalid checksum path".into()); }
    let meta = fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    regular(&meta)?; Ok(meta.is_dir())
}
fn inventory(path: &Path, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    let ctl = opts.control.clone().unwrap_or_default(); let mut result = Vec::new();
    let mut bytes = 0usize;
    for item in fs::read_dir(path).map_err(|e|e.to_string())? {
        ctl.check()?; let item = item.map_err(|e|e.to_string())?;
        let meta = fs::symlink_metadata(item.path()).map_err(|e|e.to_string())?; regular(&meta)?;
        let name = item.file_name();
        reserve(&mut bytes, std::mem::size_of::<Entry>() + name.as_encoded_bytes().len())?;
        result.push(Entry { name, directory:meta.is_dir(), size:if meta.is_file(){meta.len()}else{0}, modified:meta.modified().ok() });
    }
    result.sort_by(|a,b|a.name.cmp(&b.name)); Ok(result)
}
impl Inputs {
    pub fn collect(paths: &[String], opts: &EngineOptions) -> Result<Self,String> {
        let mut out = Self { files:Vec::new(),has_directories:false,directories:Vec::new(),_ancestors:Vec::new(),_leaves:Vec::new(),seen_files:Default::default(),seen_dirs:Default::default(),metadata_bytes:0 };
        let kinds=paths.iter().map(|p|is_directory(p)).collect::<Result<Vec<_>,_>>()?;
        if !kinds.iter().any(|d|*d) { out.files=paths.to_vec();return Ok(out); }
        for raw in paths {
            opts.control.clone().unwrap_or_default().check()?;
            if !is_directory(raw)? {
                let path=std::path::absolute(raw).map_err(|e|e.to_string())?;
                let pin=crate::engine_stream::pin(&path)?;
                let key=fs::canonicalize(&path).map_err(|e|e.to_string())?;
                if out.seen_files.insert(key) {
                    out.reserve_path(&path)?;let (leaf,parents)=pin.into_handle_parts();out._leaves.push(leaf);out._ancestors.extend(parents);out.files.push(raw.clone());
                }
                continue;
            }
            out.has_directories = true;
            let path = std::path::absolute(raw).map_err(|e|e.to_string())?;
            crate::engine::no_links(&path)?;
            #[cfg(windows)] {
                use std::os::windows::fs::OpenOptionsExt;
                out._ancestors.extend(crate::engine_stream::pin_parent_names(&path).map_err(|e|e.to_string())?);
                let file=File::options().access_mode(0x81).share_mode(1).custom_flags(0x02200000).open(&path).map_err(|e|e.to_string())?;
                out.walk(path,file,opts,0)?;
            }
            #[cfg(not(windows))] { let file=File::open(&path).map_err(|e|e.to_string())?; out.walk(path,file,opts,0)?; }
        }
        Ok(out)
    }
    fn reserve_path(&mut self, path: &Path) -> Result<(), String> {
        // Canonical name, display name, directory inventory and pinned handles.
        reserve(&mut self.metadata_bytes, 384 + path.as_os_str().as_encoded_bytes().len().saturating_mul(4))
    }
    fn walk(&mut self,path:PathBuf,file:File,opts:&EngineOptions,depth:usize)->Result<(),String> {
        opts.control.clone().unwrap_or_default().check()?;
        if depth > MAX_DEPTH { return Err("校验目录超过 256 层，请直接选择较深的子目录".into()); }
        let metadata=file.metadata().map_err(|e|e.to_string())?; regular(&metadata)?;
        if !metadata.is_dir() { return Err("checksum directory changed".into()); }
        if !self.seen_dirs.insert(fs::canonicalize(&path).map_err(|e|e.to_string())?) {return Ok(());}
        self.reserve_path(&path)?;
        let entries=inventory(&path,opts)?;
        for entry in &entries {
            opts.control.clone().unwrap_or_default().check()?;
            let child=path.join(&entry.name);
            if entry.directory {
                #[cfg(windows)] let handle=crate::private_stage::open_checksum_child(&file,&entry.name,true)?;
                #[cfg(not(windows))] let handle=File::open(&child).map_err(|e|e.to_string())?;
                self.walk(child,handle,opts,depth+1)?;
            } else {
                #[cfg(windows)] let leaf=crate::private_stage::open_checksum_child(&file,&entry.name,false)?;
                #[cfg(not(windows))] let leaf=File::open(&child).map_err(|e|e.to_string())?;
                let meta=leaf.metadata().map_err(|e|e.to_string())?;regular(&meta)?;
                if !meta.is_file(){return Err("checksum file changed".into());}
                let key=fs::canonicalize(&child).map_err(|e|e.to_string())?;
                if self.seen_files.insert(key) {
                    self.reserve_path(&child)?;self._leaves.push(leaf);
                    self.files.push(child.into_os_string().into_string().map_err(|_|"invalid checksum file name")?);
                }
            }
        }
        self.directories.push(Directory { path,_file:file,entries }); Ok(())
    }
    pub fn verify(&self,opts:&EngineOptions)->Result<(),String> {
        for dir in &self.directories {
            opts.control.clone().unwrap_or_default().check()?;
            if inventory(&dir.path,opts)?!=dir.entries { return Err("checksum directory changed during hashing".into()); }
        }
        Ok(())
    }
}
