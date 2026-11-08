//! A private working copy of one archive member, owned by its nested tab.
use super::{control, edit_source::EditSource, EngineOptions, JobControl, ShellDragDirectory};
use crate::{engine_stream, operations};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub struct NestedArchive {
    directory: ShellDragDirectory,
    path: PathBuf,
    parent: PathBuf,
    parent_identity: File,
    member: String,
    original_hash: [u8; 32],
}

fn utf8(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "invalid archive path".to_string())
}

fn digest(file: &mut File, control: &JobControl) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        control.check()?;
        let size = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if size == 0 {
            return Ok(hash.finalize().into());
        }
        hash.update(&buffer[..size]);
    }
}

impl NestedArchive {
    pub fn open(parent: &str, member: &str, options: &EngineOptions) -> Result<Self, String> {
        let parent = super::absolute(parent)?;
        let relative = super::validate_name(member)?;
        if member.ends_with(['/', '\\']) {
            return Err("a nested archive must be a file".into());
        }
        // While extracting, deny replacement. After opening, keep only a
        // metadata reference so other applications may edit the outer archive.
        let source = engine_stream::pin(&parent)?;
        let parent_identity = File::options()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(&parent)
            .map_err(|e| e.to_string())?;
        if super::directory_identity(&parent_identity)? != super::directory_identity(&source)? {
            return Err("parent archive changed while opening member".into());
        }
        let directory = ShellDragDirectory::create_named("qing-zip-member-")?;
        let extract = EngineOptions {
            selected: vec![member.to_owned()],
            overwrite: "error".into(),
            ..options.clone()
        };
        let count = operations::extract_archive(utf8(&parent)?, utf8(directory.path())?, &extract)?;
        let path = directory.path().join(relative);
        if count != 1 {
            return Err("expected exactly one nested archive member".into());
        }
        let mut content = engine_stream::pin(&path)?;
        let original_hash = digest(&mut content, &control(options))?;
        drop(content);
        Ok(Self {
            directory,
            path,
            parent,
            parent_identity,
            member: member.to_owned(),
            original_hash,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Commit only if the same parent still contains the bytes originally opened.
    /// Any error leaves the working copy available for retry or Save As.
    pub fn save(&mut self, options: &EngineOptions) -> Result<(), String> {
        let job = control(options);
        let mut working = engine_stream::pin(&self.path)?;
        let next_hash = digest(&mut working, &job)?;
        if next_hash == self.original_hash {
            return Ok(());
        }
        let _policy = crate::machine_policy::scope();
        let _lock = super::update_lock(&self.parent, &job)?;
        super::zip_volume_view::require_editable(&self.parent)?;
        let edit = EditSource::open(&self.parent)?;
        let verify = Self::open(utf8(&self.parent)?, &self.member, options)?;
        if super::directory_identity(&verify.parent_identity)?
            != super::directory_identity(&self.parent_identity)?
            || verify.original_hash != self.original_hash
        {
            return Err("parent archive or its member changed since opening".into());
        }
        drop(verify);
        let output = edit.output()?;
        let staged = output.path();
        let mut args = super::base("a", options)?;
        let archive = if edit.direct_zip(&self.parent)? {
            args.push("-u-".into());
            args.push(format!("-up1q1r2x2y2z2w2!{}", utf8(staged)?));
            &self.parent
        } else {
            fs::copy(&self.parent, staged).map_err(|e| e.to_string())?;
            args.push("-up1q1r2x2y2z2w2".into());
            staged
        };
        args.extend([
            "--".into(),
            utf8(archive)?.to_owned(),
            self.member.replace('\\', "/"),
        ]);
        super::run(&args, Some(self.directory.path()), options, None)?;
        let verified = Self::open(utf8(staged)?, &self.member, options)?;
        if verified.original_hash != next_hash {
            return Err("nested archive write-back verification failed".into());
        }
        drop(verified);
        let next_identity = File::options()
            .access_mode(0x80)
            .share_mode(7)
            .open(staged)
            .map_err(|e| e.to_string())?;
        job.check()?;
        edit.commit(output)?;
        self.parent_identity = next_identity;
        self.original_hash = next_hash;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn zip(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in members {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    fn member(path: &Path, name: &str) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn nested_zip_writeback_preserves_sibling_and_allows_a_second_save() {
        let photo = zip(&[("picture.txt", b"original")]);
        let outer = zip(&[
            ("left/same.zip", &photo),
            ("right/same.zip", &photo),
            ("extra", b"keep"),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("outer.zip");
        fs::write(&file, outer).unwrap();
        let opts = EngineOptions::default();
        let mut tab = NestedArchive::open(utf8(&file).unwrap(), "left/same.zip", &opts).unwrap();
        let private_root = tab.directory.path().to_owned();
        for text in [b"updated1", b"updated2"] {
            let changed = zip(&[("picture.txt", text)]);
            fs::write(tab.path(), &changed).unwrap();
            tab.save(&opts).unwrap();
            assert_eq!(member(&file, "left/same.zip"), changed);
            assert_eq!(member(&file, "right/same.zip"), photo);
            assert_eq!(member(&file, "extra"), b"keep");
        }
        drop(tab);
        assert!(!private_root.exists());
    }

    #[test]
    fn externally_changed_outer_member_prevents_writeback_and_keeps_edited_copy() {
        let old = zip(&[("one", b"original")]);
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("outer.zip");
        fs::write(&file, zip(&[("a.zip", &old)])).unwrap();
        let opts = EngineOptions::default();
        let mut tab = NestedArchive::open(utf8(&file).unwrap(), "a.zip", &opts).unwrap();
        let foreign = zip(&[("one", b"external")]);
        let mine = zip(&[("one", b"my edit")]);
        fs::write(&file, zip(&[("a.zip", &foreign)])).unwrap();
        fs::write(tab.path(), &mine).unwrap();
        assert!(tab
            .save(&opts)
            .unwrap_err()
            .contains("changed since opening"));
        assert_eq!(member(&file, "a.zip"), foreign);
        assert_eq!(fs::read(tab.path()).unwrap(), mine);
    }
}
