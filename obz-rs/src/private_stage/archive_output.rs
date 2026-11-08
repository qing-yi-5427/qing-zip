use super::*;

// Keep the console's two-level NTFS inheritance and storage layout. Every
// operation refers to handles we created; an injected foreign entry is never
// traversed or removed by cleanup.
pub(crate) struct ArchiveOutput {
    temp: Option<PrivateOutputFile>,
    root: Option<File>,
    ordinary: Option<File>,
    parent: File,
    directory: PathBuf,
}
impl ArchiveOutput {
    pub(crate) fn try_new(path: &Path, parent: &File) -> Result<Option<Self>, String> {
        let (directory, root, parent) = match create_root_with_descriptor(
            path,
            parent,
            ".qing-zip-create-",
            0x1300a3,
            3,
            std::ptr::null_mut(),
        ) {
            Ok(stage) => stage,
            // No object was created. The console may work with fewer rights.
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) => return Ok(None),
            Err(e) => return Err(format!("create ZIP staging directory: {e}")),
        };
        let mut output = Self {
            temp: None,
            root: Some(root),
            ordinary: None,
            parent,
            directory,
        };
        // FILE_CREATE and DELETE_ON_CLOSE; this empty file contains no
        // archive data. Keep its native inherited ACL as the reference.
        match open_private_child(
            output.root.as_ref().unwrap(),
            OsStr::new("permissions"),
            ChildAccess::ArchiveReference,
        ) {
            Ok(reference) => output.ordinary = Some(reference),
            Err(_) => {
                output.cleanup()?;
                return Ok(None);
            }
        }
        match output::create_archive_file(&output.directory, output.root.as_ref().unwrap()) {
            Ok(file) => output.temp = Some(file),
            Err(reason) => {
                return Err(match output.cleanup() {
                    Ok(()) => reason,
                    Err(cleanup) => format!("{reason}; staging cleanup failed: {cleanup}"),
                })
            }
        }
        Ok(Some(output))
    }
    pub(crate) fn file(&self) -> &File {
        self.temp.as_ref().unwrap().as_file()
    }
    pub(crate) fn publish(
        &mut self,
        name: &OsStr,
        before_commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.temp.as_mut().unwrap().publish_archive(
            &self.parent,
            self.ordinary.as_ref().unwrap(),
            name,
            before_commit,
        )
    }
    pub(crate) fn cleanup(&mut self) -> Result<(), String> {
        if let Some(mut file) = self.temp.take() {
            file.abort()?;
        }
        self.ordinary.take();
        if let Some(root) = self.root.take() {
            let mut io = IoStatus {
                status: 0,
                information: 0,
            };
            let delete = 1u8;
            let result = unsafe {
                NtSetInformationFile(
                    root.as_raw_handle(),
                    (&mut io as *mut IoStatus).cast(),
                    (&delete as *const u8).cast(),
                    1,
                    13,
                )
            };
            if result < 0 {
                return Err(format!(
                    "cannot remove owned empty archive stage {}: {}",
                    self.directory.display(),
                    std::io::Error::from_raw_os_error(
                        unsafe { RtlNtStatusToDosError(result) } as i32
                    )
                ));
            }
        }
        Ok(())
    }
}
impl Drop for ArchiveOutput {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
