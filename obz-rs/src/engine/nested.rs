//! A private working copy of one archive member, owned by its nested tab.
use super::{control, edit_source::EditSource, EngineOptions, JobControl, ShellDragDirectory};
use crate::{engine_stream, operations};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkingFileVersion {
    identity: (u32, u64),
    size: u64,
    created: u64,
    modified: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RecoveryDigest {
    // Index entries created before the algorithm field used SHA-256.
    #[default]
    Sha256,
    Blake3,
}

impl RecoveryDigest {
    fn digest(self, file: &mut impl Read, control: &JobControl) -> Result<[u8; 32], String> {
        match self {
            Self::Sha256 => {
                let mut hash = Sha256::new();
                hash_chunks(file, control, |bytes| hash.update(bytes))?;
                Ok(hash.finalize().into())
            }
            Self::Blake3 => {
                let mut hash = blake3::Hasher::new();
                hash_chunks(file, control, |bytes| { hash.update(bytes); })?;
                Ok(*hash.finalize().as_bytes())
            }
        }
    }
}
impl WorkingFileVersion {
    fn from_file(file: &File) -> Result<Self, String> {
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & 0x4400 != 0 {
            return Err("working copy is not an ordinary file".into());
        }
        Ok(Self {
            identity: super::directory_identity(file)?,
            size: metadata.file_size(),
            created: metadata.creation_time(),
            modified: metadata.last_write_time(),
        })
    }
}

/// Intent pins one private TEMP workspace. Only a published revision can make
/// the file eligible for export after a restart.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCopy {
    workspace: PathBuf,
    workspace_identity: (u32, u64),
    original_hash: [u8; 32],
    committed: Option<(WorkingFileVersion, [u8; 32])>,
    #[serde(default)]
    digest: RecoveryDigest,
}
pub struct PinnedRecoveryCopy {
    _source: engine_stream::PinnedSource,
}
impl RecoveryCopy {
    pub fn record_commit(&mut self, path: &Path, revision: &ArchiveRevision) -> Result<bool, String> {
        let mut source = engine_stream::pin(path)?;
        let committed = WorkingFileVersion::from_file(&revision.after)?;
        if WorkingFileVersion::from_file(&source)? != committed {
            return Ok(false);
        }
        let hash = self.digest.digest(&mut *source, &JobControl::new())?;
        self.committed = (hash != self.original_hash).then_some((committed, hash));
        Ok(true)
    }

    pub fn has_commit(&self) -> bool {
        self.committed.is_some()
    }

    pub fn pin_changed(&self, path: &Path, member: &str, verify_bytes: bool) -> Result<Option<PinnedRecoveryCopy>, String> {
        let Some((committed, hash)) = &self.committed else { return Ok(None); };
        let relative = super::validate_name(member)?;
        if path != self.workspace.join(relative)
            || !self.workspace.file_name().and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("qing-zip-member-"))
        {
            return Ok(None);
        }
        let mut source = engine_stream::pin(path)?;
        let workspace = engine_stream::pin_directory_metadata(&self.workspace)?;
        if fs::canonicalize(&self.workspace).ok().as_deref().and_then(Path::parent)
            != Some(fs::canonicalize(std::env::temp_dir()).map_err(|e| e.to_string())?.as_path())
            || super::directory_identity(&workspace)? != self.workspace_identity
            || WorkingFileVersion::from_file(&source)? != *committed
        {
            return Ok(None);
        }
        if verify_bytes && self.digest.digest(&mut *source, &JobControl::new())? != *hash {
            return Ok(None);
        }
        Ok(Some(PinnedRecoveryCopy { _source: source }))
    }
}

/// Check bounded ZIP metadata before arming recovery. This includes encrypted
/// members elsewhere in a parent even when opening the selected member did not
/// require a password.
pub fn ordinary_recovery_zip(path: &Path) -> bool {
    engine_stream::pin(path).is_ok_and(|mut source| {
        ordinary_recovery_zip_from(path, &mut source).unwrap_or(false)
    })
}

fn ordinary_recovery_zip_from(path: &Path, source: &mut File) -> Result<bool, String> {
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            || path.with_extension("z01").exists()
            || crate::machine_policy::scope().restricted()
        {
            return Ok(false);
        }
        if source.metadata().map_err(|e| e.to_string())?.file_attributes() & 0x4000 != 0 {
            return Ok(false);
        }
        let Some(record) = super::zip_volume_view::single_disk_record(source)? else {
            return Ok(false);
        };
        let entries = usize::from(record.entries);
        if record.zip64 || entries > super::MEMORY_ZIP_ENTRY_LIMIT
            || record.directory_size > 8 * 1024 * 1024
            || u64::from(record.directory) + u64::from(record.directory_size) != record.end_record
        {
            return Ok(false);
        }
        let archive = zip::ZipArchive::new(std::io::BufReader::with_capacity(64 * 1024, source))
            .map_err(|e| e.to_string())?;
        Ok(archive.len() == entries
            && (0..entries).all(|index| archive.by_index_data(index).is_ok_and(|member| !member.encrypted())))
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
    ordinary_source: bool,
}

fn utf8(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "invalid archive path".to_string())
}

fn hash_chunks(file: &mut impl Read, control: &JobControl, mut update: impl FnMut(&[u8])) -> Result<(), String> {
    let mut buffer = vec![0; 256 * 1024];
    loop {
        control.check()?;
        let size = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if size == 0 {
            return Ok(());
        }
        update(&buffer[..size]);
    }
}

// Bind one validated ZIP namespace to the held source and send the exact
// member to BLAKE3. Candidate publication also verifies all unrelated members.
// Other decoders retain the private extraction path and its verification.
fn zip_member_digest(
    path: &Path,
    member: &str,
    options: &EngineOptions,
    source: &mut engine_stream::PinnedSource,
    whole_archive: bool,
) -> Result<Option<[u8; 32]>, String> {
    use std::io::{Seek, SeekFrom};

    let policy = crate::machine_policy::scope();
    if policy.restricted()
        || options.password.is_some()
        || options.codepage.is_some()
        || !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        || path.with_extension("z01").exists()
    {
        return Ok(None);
    }
    let job = control(options);
    let Some(record) = super::zip_volume_view::single_disk_record(source)? else { return Ok(None); };
    if record.zip64 || !super::ordinary_zip_file_layout_count(source, usize::from(record.entries), &job)? {
        return Ok(None);
    }
    let mut list_options = options.clone();
    list_options.selected.clear();
    let (entries, _) = super::list_raw_with_source(utf8(path)?, &list_options, source)?;
    super::expected_tree(&entries)?;
    source.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
    let mut archive = match zip::ZipArchive::new(std::io::BufReader::with_capacity(256 * 1024, &mut **source)) {
        Ok(archive) => archive,
        Err(_) => return Ok(None),
    };
    let Some(order) = super::verified_zip_order(&mut archive, &entries, &job)? else {
        return Ok(None);
    };
    let wanted = super::path_key(&super::validate_name(member)?);
    let mut buffer = vec![0; 256 * 1024];
    let mut matching_hash = None;
    for (index, entry) in order.into_iter().map(|index| &entries[index]).enumerate() {
        job.check()?;
        let selected = !entry.is_dir && super::path_key(&super::validate_name(&entry.name)?) == wanted;
        if !whole_archive && !selected { continue; }
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let mut whole = crc32fast::Hasher::new();
        let mut selected_hash = selected.then(blake3::Hasher::new);
        let mut size = 0u64;
        loop {
            job.check()?;
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 { break; }
            size = size.checked_add(count as u64).ok_or("ZIP decoded size overflow")?;
            if size > entry.size { return Err("ZIP decoded length mismatch".into()); }
            whole.update(&buffer[..count]);
            if let Some(hash) = &mut selected_hash {
                hash.update(&buffer[..count]);
            }
        }
        if size != entry.size || whole.finalize() != file.crc32() {
            return Err("ZIP size or CRC mismatch".into());
        }
        if let Some(hash) = selected_hash {
            matching_hash = Some(*hash.finalize().as_bytes());
        }
    }
    job.check()?;
    policy.validate_output()?;
    matching_hash.map(Some).ok_or_else(|| "expected exactly one nested archive member".into())
}

impl NestedArchive {
    pub fn open(parent: &str, member: &str, options: &EngineOptions) -> Result<Self, String> {
        Self::open_in(parent, member, options, None, false)
    }
    pub fn open_with_recovery(parent: &str, member: &str, options: &EngineOptions) -> Result<Self, String> {
        Self::open_in(parent, member, options, None, true)
    }
    pub fn open_external(
        parent: &str,
        member: &str,
        options: &EngineOptions,
        root: &Path,
    ) -> Result<Self, String> {
        Self::open_in(parent, member, options, Some(root), false)
    }
    fn open_in(
        parent: &str,
        member: &str,
        options: &EngineOptions,
        root: Option<&Path>,
        inspect_for_recovery: bool,
    ) -> Result<Self, String> {
        let parent = super::absolute(parent)?;
        let relative = super::validate_name(member)?;
        if member.ends_with(['/', '\\']) {
            return Err("a nested archive must be a file".into());
        }
        // While extracting, deny replacement. After opening, keep only a
        // metadata reference so other applications may edit the outer archive.
        let mut source = engine_stream::pin(&parent)?;
        let parent_identity = metadata_reference(&parent)?;
        if super::directory_identity(&parent_identity)? != super::directory_identity(&source)? {
            return Err("parent archive changed while opening member".into());
        }
        let ordinary_source = inspect_for_recovery && options.password.is_none()
            && ordinary_recovery_zip_from(&parent, &mut source).unwrap_or(false);
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
        let original_hash = RecoveryDigest::Blake3.digest(&mut *content, &control(options))?;
        drop(content);
        Ok(Self {
            directory,
            path,
            parent,
            parent_identity,
            member: member.to_owned(),
            original_hash,
            ordinary_source,
        })
    }

    fn member_proof(
        parent: &Path,
        member: &str,
        options: &EngineOptions,
        source: &mut engine_stream::PinnedSource,
        whole_archive: bool,
    ) -> Result<[u8; 32], String> {
        if let Some(hash) = zip_member_digest(parent, member, options, source, whole_archive)? {
            Ok(hash)
        } else {
            Ok(Self::open(utf8(parent)?, member, options)?.original_hash)
        }
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
        Ok(RecoveryDigest::Blake3.digest(&mut *working, &control(options))? != self.original_hash)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn recovery_copy(&self) -> Result<RecoveryCopy, String> {
        let workspace = self.directory.pin_for_read()?;
        Ok(RecoveryCopy {
            workspace: self.directory.path().to_owned(),
            workspace_identity: super::directory_identity(&workspace)?,
            original_hash: self.original_hash,
            committed: None,
            digest: RecoveryDigest::Blake3,
        })
    }
    pub fn has_ordinary_source(&self) -> bool {
        self.ordinary_source
    }

    pub fn check_parent_current(&self) -> Result<(), String> {
        let current = engine_stream::pin(&self.parent)?;
        if super::directory_identity(&current)? != super::directory_identity(&self.parent_identity)? {
            return Err("parent archive changed while opening member".into());
        }
        Ok(())
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
        let mut current = engine_stream::pin(&self.parent)?;
        if super::directory_identity(&current)? != super::directory_identity(&revision.after)? {
            return Ok(false);
        }
        if Self::member_proof(&self.parent, &self.member, options, &mut current, false)? != self.original_hash {
            return Ok(false);
        }
        self.parent_identity = revision.after.try_clone().map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// Commit only if the same parent still contains the bytes originally opened.
    /// Any error leaves the working copy available for retry or Save As.
    pub fn save(&mut self, options: &EngineOptions) -> Result<(), String> {
        let job = control(options);
        let mut working = engine_stream::pin(&self.path)?;
        let next_hash = RecoveryDigest::Blake3.digest(&mut *working, &job)?;
        if next_hash == self.original_hash {
            return Ok(());
        }
        let _policy = crate::machine_policy::scope();
        let _lock = super::update_lock(&self.parent, &job)?;
        super::zip_volume_view::require_editable(&self.parent)?;
        let edit = EditSource::open_controlled(&self.parent, &job)?;
        {
            let mut current = engine_stream::pin(&self.parent)?;
            if super::directory_identity(&current)? != super::directory_identity(&self.parent_identity)?
                || Self::member_proof(&self.parent, &self.member, options, &mut current, false)? != self.original_hash
            {
                return Err("parent archive or its member changed since opening".into());
            }
        }
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
        {
            let mut proposed = engine_stream::pin(staged)?;
            if Self::member_proof(staged, &self.member, options, &mut proposed, true)? != next_hash {
                return Err("nested archive write-back verification failed".into());
            }
        }
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
    fn nested_recovery_requires_changed_bytes_committed_to_the_owned_working_copy() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("outer.zip");
        let original = zip(&[("before.txt", b"ordinary fixture")]);
        fs::write(&parent, zip(&[("inner.zip", &original)])).unwrap();
        let tab = NestedArchive::open(utf8(&parent).unwrap(), "inner.zip", &Default::default()).unwrap();
        let mut recovery = tab.recovery_copy().unwrap();
        let mut legacy = recovery.clone();
        legacy.digest = RecoveryDigest::Sha256;
        legacy.original_hash = Sha256::digest(&original).into();
        let path = tab.path();
        assert!(recovery.pin_changed(path, "inner.zip", true).unwrap().is_none());
        let identical = ArchiveRevision::new(path, path).unwrap();
        assert!(recovery.record_commit(path, &identical).unwrap());
        assert!(!recovery.has_commit());
        drop(identical);

        let mut options = EngineOptions::default();
        let receipt = EditReceipt::capture(&mut options);
        operations::execute_request(operations::Request {
            operation: "rename".into(), path: utf8(path).unwrap().into(),
            inputs: vec!["before.txt".into(), "after.txt".into()], options,
            ..Default::default()
        }).unwrap();
        let committed = receipt.take().unwrap();
        assert!(recovery.record_commit(path, &committed).unwrap());
        assert!(recovery.pin_changed(path, "inner.zip", false).unwrap().is_some());
        assert!(recovery.pin_changed(path, "inner.zip", true).unwrap().is_some());
        let saved = fs::read(path).unwrap();
        assert_eq!(recovery.committed.as_ref().unwrap().1, *blake3::hash(&saved).as_bytes());
        assert!(legacy.record_commit(path, &committed).unwrap());
        assert_eq!(legacy.committed.as_ref().unwrap().1, <[u8; 32]>::from(Sha256::digest(&saved)));
        let mut legacy_json = serde_json::to_value(legacy).unwrap();
        legacy_json.as_object_mut().unwrap().remove("digest");
        let old_record: RecoveryCopy = serde_json::from_value(legacy_json).unwrap();
        assert!(old_record.pin_changed(path, "inner.zip", true).unwrap().is_some());
        drop(committed);
        assert_eq!(member(path, "after.txt"), b"ordinary fixture");
        assert_eq!(member(&parent, "inner.zip"), original);
        let mut stale = recovery.clone();
        stale.committed.as_mut().unwrap().1[0] ^= 1;
        assert!(stale.pin_changed(path, "inner.zip", false).unwrap().is_some());
        assert!(stale.pin_changed(path, "inner.zip", true).unwrap().is_none());

        // A replacement can contain the right bytes and still cannot claim
        // the identity of the object shown in the recovery list.
        let moved = path.with_file_name("earlier.zip");
        fs::rename(path, &moved).unwrap();
        fs::copy(&moved, path).unwrap();
        assert!(recovery.pin_changed(path, "inner.zip", false).unwrap().is_none());
        fs::remove_file(&moved).unwrap();
        assert_eq!(member(&parent, "inner.zip"), original);
        assert_eq!(member(path, "after.txt"), b"ordinary fixture");
    }

    #[test]
    fn nested_recovery_excludes_a_zip_with_an_unrelated_encrypted_member() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mixed.zip");
        let inner = zip(&[("normal.txt", b"fixture")]);
        let options = zip::write::SimpleFileOptions::default();
        let mut mixed = zip::ZipWriter::new(File::create(&path).unwrap());
        mixed.start_file("inner.zip", options).unwrap();
        mixed.write_all(&inner).unwrap();
        mixed.start_file("locked.txt", options.with_aes_encryption(zip::AesMode::Aes256, "test fixture")).unwrap();
        mixed.write_all(b"independent encrypted member").unwrap();
        drop(mixed.finish().unwrap());
        assert_eq!(member(&path, "inner.zip"), inner);
        assert!(!ordinary_recovery_zip(&path));
        let plain = temp.path().join("plain.zip");
        fs::write(&plain, zip(&[("inner.zip", &inner)])).unwrap();
        assert!(ordinary_recovery_zip(&plain));
    }

    #[test]
    fn ordinary_zip_member_digest_reads_the_member_without_materializing_it() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("normal.zip");
        fs::write(&parent, zip(&[("entry.bin", b"working bytes"), ("sibling", b"sibling")])).unwrap();
        let mut pinned = engine_stream::pin(&parent).unwrap();
        let expected = *blake3::hash(b"working bytes").as_bytes();
        assert_eq!(
            zip_member_digest(&parent, "entry.bin", &EngineOptions::default(), &mut pinned, false).unwrap(),
            Some(expected)
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        drop(pinned);

        // Damage the unrelated member. A rebind proves the exact requested
        // member only, while a proposed write-back must reject the whole ZIP.
        let mut bytes = fs::read(&parent).unwrap();
        let position = zip::ZipArchive::new(Cursor::new(&bytes))
            .unwrap().by_index(1).unwrap().data_start().unwrap() as usize;
        bytes[position] ^= 1;
        fs::write(&parent, bytes).unwrap();
        let mut pinned = engine_stream::pin(&parent).unwrap();
        assert_eq!(
            zip_member_digest(&parent, "entry.bin", &EngineOptions::default(), &mut pinned, false).unwrap(),
            Some(expected)
        );
        assert!(zip_member_digest(&parent, "entry.bin", &EngineOptions::default(), &mut pinned, true).is_err());
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
