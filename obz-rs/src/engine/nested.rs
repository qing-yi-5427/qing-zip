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

/// The exact filesystem objects on both sides of one successful archive edit.
#[derive(Debug)]
pub struct ArchiveRevision {
    before: File,
    after: File,
}

fn metadata_reference(path: &Path) -> Result<File, String> {
    File::options()
        .access_mode(0x80)
        .share_mode(7)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|error| error.to_string())
}

impl ArchiveRevision {
    pub(super) fn new(before: &Path, after: &Path) -> Result<Self, String> {
        Ok(Self {
            before: metadata_reference(before)?,
            after: metadata_reference(after)?,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct EditReceipt(std::sync::Arc<std::sync::Mutex<Option<ArchiveRevision>>>);
impl EditReceipt {
    pub fn capture(options: &mut EngineOptions) -> Self {
        let receipt = Self::default();
        options.edit_receipt = Some(receipt.clone());
        receipt
    }
    pub fn take(&self) -> Option<ArchiveRevision> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
    pub(super) fn record(&self, revision: ArchiveRevision) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(revision);
    }
}

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
        Self::open_in(parent, member, options, None)
    }
    pub fn open_external(
        parent: &str,
        member: &str,
        options: &EngineOptions,
        root: &Path,
    ) -> Result<Self, String> {
        Self::open_in(parent, member, options, Some(root))
    }
    fn open_in(
        parent: &str,
        member: &str,
        options: &EngineOptions,
        root: Option<&Path>,
    ) -> Result<Self, String> {
        let parent = super::absolute(parent)?;
        let relative = super::validate_name(member)?;
        if member.ends_with(['/', '\\']) {
            return Err("a nested archive must be a file".into());
        }
        // While extracting, deny replacement. After opening, keep only a
        // metadata reference so other applications may edit the outer archive.
        let source = engine_stream::pin(&parent)?;
        let parent_identity = metadata_reference(&parent)?;
        if super::directory_identity(&parent_identity)? != super::directory_identity(&source)? {
            return Err("parent archive changed while opening member".into());
        }
        let directory = match root {
            Some(root) => ShellDragDirectory::create_named_in(root, "edit-")?,
            None => ShellDragDirectory::create_named("qing-zip-member-")?,
        };
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

    /// External editors may hand the document to an already-running process.
    /// Retain the private copy until its caller can prove nobody still owns it.
    pub fn retain_working_copy(&self, retain: bool) {
        if retain {
            self.directory.retain_for_shell();
        } else {
            self.directory.shell_read_finished();
        }
    }
    pub fn has_changes(&self, options: &EngineOptions) -> Result<bool, String> {
        let mut working = engine_stream::pin(&self.path)?;
        Ok(digest(&mut working, &control(options))? != self.original_hash)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Follow a known successful edit by this process only if this same member
    /// survived unchanged. Other writers and stale receipts cannot rebind a tab.
    pub fn rebind_after_commit(
        &mut self,
        revision: &ArchiveRevision,
        options: &EngineOptions,
    ) -> Result<bool, String> {
        if super::directory_identity(&self.parent_identity)?
            != super::directory_identity(&revision.before)?
        {
            return Ok(false);
        }
        let current = engine_stream::pin(&self.parent)?;
        if super::directory_identity(&current)? != super::directory_identity(&revision.after)? {
            return Ok(false);
        }
        let now = Self::open(utf8(&self.parent)?, &self.member, options)?;
        if now.original_hash != self.original_hash {
            return Ok(false);
        }
        self.parent_identity = now.parent_identity;
        Ok(true)
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
        let edit = EditSource::open_controlled(&self.parent, &job)?;
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
        let next_identity = metadata_reference(staged)?;
        job.check()?;
        edit.commit_tracked(output, options)?;
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
    #[test]
    fn replacing_parent_with_identical_member_does_not_redirect_writeback() {
        let original = zip(&[("item", b"first")]);
        let temp = tempfile::tempdir().unwrap();
        let outer = temp.path().join("outer.zip");
        fs::write(&outer, zip(&[("member.zip", &original)])).unwrap();
        let opts = EngineOptions::default();
        let mut tab = NestedArchive::open(utf8(&outer).unwrap(), "member.zip", &opts).unwrap();
        let other = temp.path().join("other.zip");
        let foreign = zip(&[("member.zip", &original), ("new", b"external")]);
        fs::write(&other, &foreign).unwrap();
        fs::rename(&other, &outer).unwrap();
        fs::write(tab.path(), zip(&[("my edit", b"edit")])).unwrap();
        assert!(tab.save(&opts).is_err());
        assert_eq!(fs::read(&outer).unwrap(), foreign);
    }

    #[test]
    fn two_levels_of_nested_edits_round_trip_to_the_outer_archive() {
        let leaf = zip(&[("note.txt", b"old")]);
        let middle = zip(&[("folder/leaf.zip", &leaf), ("sibling", b"unchanged")]);
        let temp = tempfile::tempdir().unwrap();
        let outer = temp.path().join("outer.zip");
        fs::write(&outer, zip(&[("middle.zip", &middle)])).unwrap();
        let opts = EngineOptions::default();
        let mut middle = NestedArchive::open(utf8(&outer).unwrap(), "middle.zip", &opts).unwrap();
        let mut child =
            NestedArchive::open(utf8(middle.path()).unwrap(), "folder/leaf.zip", &opts).unwrap();
        let updated = zip(&[("note.txt", b"new")]);
        fs::write(child.path(), &updated).unwrap();
        child.save(&opts).unwrap();
        middle.save(&opts).unwrap();
        drop(child);
        drop(middle);
        let reopened = NestedArchive::open(utf8(&outer).unwrap(), "middle.zip", &opts).unwrap();
        assert_eq!(member(reopened.path(), "folder/leaf.zip"), updated);
        assert_eq!(member(reopened.path(), "sibling"), b"unchanged");
    }

    #[test]
    fn corrupt_unselected_member_prevents_nested_open() {
        let nested = zip(&[("entry", b"verified")]);
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive.start_file("good.zip", options).unwrap();
        archive.write_all(&nested).unwrap();
        archive.start_file("other", options).unwrap();
        let signature = b"unique corrupt sibling payload";
        archive.write_all(signature).unwrap();
        let mut bytes = archive.finish().unwrap().into_inner();
        let at = bytes
            .windows(signature.len())
            .position(|w| w == signature)
            .unwrap();
        bytes[at] ^= 1;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.zip");
        fs::write(&path, &bytes).unwrap();
        assert!(
            NestedArchive::open(utf8(&path).unwrap(), "good.zip", &EngineOptions::default())
                .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn authenticated_outer_zip_keeps_member_encrypted_after_nested_writeback() {
        let temp = tempfile::tempdir().unwrap();
        let inner = temp.path().join("inner.zip");
        let parent = temp.path().join("encrypted.zip");
        fs::write(&inner, zip(&[("note", b"old")])).unwrap();
        let parent_options = EngineOptions {
            password: Some("test-outer".into()),
            encryption: Some("AES256".into()),
            ..Default::default()
        };
        crate::engine::create(
            utf8(&parent).unwrap(),
            &[utf8(&inner).unwrap().to_owned()],
            "zip",
            &parent_options,
        )
        .unwrap();
        let credentials = EngineOptions {
            password: Some("test-outer".into()),
            ..Default::default()
        };
        let wrong = EngineOptions {
            password: Some("wrong".into()),
            ..Default::default()
        };
        assert!(NestedArchive::open(utf8(&parent).unwrap(), "inner.zip", &wrong).is_err());
        let mut tab =
            NestedArchive::open(utf8(&parent).unwrap(), "inner.zip", &credentials).unwrap();
        fs::write(tab.path(), zip(&[("note", b"updated")])).unwrap();
        tab.save(&credentials).unwrap();
        let entries = operations::list_archive(utf8(&parent).unwrap(), &credentials).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].encrypted && entries[0].method.contains("AES-256"),
            "{:?}",
            entries[0]
        );
        assert!(NestedArchive::open(utf8(&parent).unwrap(), "inner.zip", &wrong).is_err());
        let valid = NestedArchive::open(utf8(&parent).unwrap(), "inner.zip", &credentials).unwrap();
        assert_eq!(member(valid.path(), "note"), b"updated");
    }
    #[test]
    fn siblings_can_follow_a_verified_edit_but_never_a_replacement_after_that_edit() {
        let original = zip(&[("note", b"original")]);
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent.zip");
        fs::write(&parent, zip(&[("a.zip", &original), ("b.zip", &original)])).unwrap();
        let base = EngineOptions::default();
        let mut first = NestedArchive::open(utf8(&parent).unwrap(), "a.zip", &base).unwrap();
        let mut sibling = NestedArchive::open(utf8(&parent).unwrap(), "b.zip", &base).unwrap();
        let mut stale = NestedArchive::open(utf8(&parent).unwrap(), "b.zip", &base).unwrap();
        let mut options = EngineOptions::default();
        let receipt = EditReceipt::capture(&mut options);
        fs::write(first.path(), zip(&[("note", b"first edit")])).unwrap();
        first.save(&options).unwrap();
        let revision = receipt.take().unwrap();
        assert!(sibling.rebind_after_commit(&revision, &base).unwrap());
        let second = zip(&[("note", b"second edit")]);
        fs::write(sibling.path(), &second).unwrap();
        sibling.save(&base).unwrap();
        assert_eq!(member(&parent, "b.zip"), second);
        let foreign = temp.path().join("foreign.zip");
        fs::write(
            &foreign,
            zip(&[("b.zip", &original), ("foreign", b"new owner")]),
        )
        .unwrap();
        fs::rename(foreign, &parent).unwrap();
        assert!(!stale.rebind_after_commit(&revision, &base).unwrap());
    }

    #[test]
    fn ordinary_parent_comment_receipt_preserves_open_member_binding() {
        let original = zip(&[("note", b"old")]);
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent.zip");
        fs::write(&parent, zip(&[("a.zip", &original)])).unwrap();
        let mut opts = EngineOptions::default();
        let mut tab = NestedArchive::open(utf8(&parent).unwrap(), "a.zip", &opts).unwrap();
        let receipt = EditReceipt::capture(&mut opts);
        crate::archive_comment::set(utf8(&parent).unwrap(), "parent change", &opts).unwrap();
        assert!(tab
            .rebind_after_commit(&receipt.take().unwrap(), &opts)
            .unwrap());
        fs::write(tab.path(), zip(&[("note", b"new")])).unwrap();
        tab.save(&opts).unwrap();
        assert_eq!(
            crate::archive_comment::get(utf8(&parent).unwrap()).unwrap(),
            "parent change"
        );
    }
}
