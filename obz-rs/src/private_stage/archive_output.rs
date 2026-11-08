use super::*;

// Keep the console's two-level NTFS inheritance and storage layout. Every
// operation refers to handles we created; an injected foreign entry is never
// traversed or removed by cleanup.
pub(crate) struct ArchiveOutput {
    temp: Option<PrivateOutputFile>,
    root: Option<File>,
    parent: File,
    directory: PathBuf,
}
impl ArchiveOutput {
    pub(crate) fn new(path: &Path, parent: &File) -> Result<Self, String> {
        let (directory, root, parent) = create_root_with_descriptor(
            path,
            parent,
            ".qing-zip-create-",
            0x1300a3,
            3,
            std::ptr::null_mut(),
        )?;
        let mut output = Self {
            temp: None,
            root: Some(root),
            parent,
            directory,
        };
        output.temp = Some(create_output(
            &output.directory,
            output.root.as_ref().unwrap(),
        )?);
        Ok(output)
    }
    pub(crate) fn file(&self) -> &File {
        self.temp.as_ref().unwrap().as_file()
    }
    pub(crate) fn publish(&mut self, name: &OsStr) -> Result<(), String> {
        self.temp
            .as_mut()
            .unwrap()
            .publish_archive(&self.parent, name)
    }
    pub(crate) fn cleanup(&mut self) -> Result<(), String> {
        if let Some(mut file) = self.temp.take() {
            file.abort()?;
        }
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
