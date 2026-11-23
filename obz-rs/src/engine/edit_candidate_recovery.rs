//! Best-effort disposal before an ordinary ZIP update claims its public original.
//! The separate edit journal handles the transaction after that claim.
use super::*;
use crate::engine::JobControl;
use std::io::{Read, Write};

const DIRECTORY: &str = "edit-candidates-v1";
const RECORD_LIMIT: u64 = 256 * 1024;
const RECORDS_PER_LOOKUP: usize = 64;
const OPEN_REPARSE_POINT: u32 = 0x00200000;
const DELETE: u32 = 0x00010000;
const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;

#[cfg(test)]
thread_local! {
    static TEST_PROFILE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn index() -> Option<PathBuf> {
    #[cfg(test)]
    return TEST_PROFILE.with(|profile| profile.borrow().as_ref().map(|p| p.join(DIRECTORY)));
    #[cfg(not(test))]
    {
        let vault = crate::services::Vault::default_path().ok()?;
        let directory = vault.parent()?.join(DIRECTORY);
        directory.is_absolute().then_some(directory)
    }
}

#[derive(PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Identity {
    id: FileId,
    birth: u64,
}

impl Identity {
    fn of(file: &File) -> Result<Self, String> {
        Ok(Self {
            id: stable_id(file)?,
            birth: file.metadata().map_err(|e| e.to_string())?.creation_time(),
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    receipt: Identity,
    bucket: Identity,
    owns_bucket: bool,
    original: Identity,
    original_name: Vec<u16>,
    parent: Identity,
    stage_name: String,
    stage: Identity,
    output: Identity,
    candidate: Identity,
    length: u64,
    digest: [u8; 32],
}

fn key(original: &Identity) -> Result<String, String> {
    let serialized = serde_json::to_vec(original).map_err(|e| e.to_string())?;
    Ok(blake3::hash(&serialized).to_hex().to_string())
}

pub(super) struct Lease {
    file: Option<File>,
    bucket: Bucket,
    owns_bucket: bool,
    #[cfg(test)]
    receipt: PathBuf,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = DisposableZipCandidate::set_disposition(&file, true);
            drop(file);
            if self.owns_bucket {
                self.bucket.remove_if_empty();
            }
        }
    }
}

struct Bucket {
    path: PathBuf,
    identity: Identity,
}

impl Bucket {
    fn capture(path: &Path) -> Result<Self, String> {
        Ok(Self {
            path: path.to_owned(),
            identity: Identity::of(&pin_directory(path)?)?,
        })
    }

    fn remove_if_empty(&self) {
        let _ = (|| -> Result<(), String> {
            let parent = self.path.parent().ok_or("missing receipt index parent")?;
            let _profile = directory_guard(parent, false)?;
            let directory = File::options()
                .access_mode(DELETE | 0x80)
                .share_mode(3)
                .custom_flags(OPEN_REPARSE_POINT | 0x02000000)
                .open(&self.path)
                .map_err(|e| e.to_string())?;
            validate_transaction_root(&directory, &self.path)?;
            if self.identity == Identity::of(&directory)? {
                // Windows refuses disposition on a non-empty directory.
                DisposableZipCandidate::set_disposition(&directory, true)?;
            }
            Ok(())
        })();
    }
}

fn bucket(directory: &Path, original: &Identity) -> Result<PathBuf, String> {
    Ok(directory.join(key(original)?))
}

fn directory_guard(directory: &Path, create: bool) -> Result<CheckedDirectories, String> {
    let mut guard = CheckedDirectories::default();
    for ancestor in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if create && !ancestor.is_dir() {
            match std::fs::create_dir(ancestor) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        guard.check(ancestor)?;
    }
    Ok(guard)
}

fn stage_label(name: &str) -> bool {
    name.strip_prefix(".qing-zip-stage-")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn digest(file: &File, length: u64, control: &JobControl) -> Result<[u8; 32], String> {
    let mut digest = blake3::Hasher::new();
    let mut position = 0;
    let mut buffer = vec![0; 256 * 1024];
    while position < length {
        control.check()?;
        let size = (length - position).min(buffer.len() as u64) as usize;
        let count = file
            .seek_read(&mut buffer[..size], position)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("candidate length changed".into());
        }
        digest.update(&buffer[..count]);
        position += count as u64;
    }
    Ok(*digest.finalize().as_bytes())
}

pub(super) fn arm(
    source: &EditSource,
    output: &EditOutput,
    candidate: &File,
    control: &JobControl,
) -> Option<Lease> {
    let directory = index()?;
    match arm_inner(&directory, source, output, candidate, control) {
        Ok(lease) => Some(lease),
        Err(error) => {
            #[cfg(test)]
            eprintln!("candidate receipt unavailable: {error}");
            let _ = error;
            None
        }
    }
}

fn arm_inner(
    directory: &Path,
    source: &EditSource,
    output: &EditOutput,
    candidate: &File,
    control: &JobControl,
) -> Result<Lease, String> {
    let original = Identity::of(&source.read)?;
    let profile = directory_guard(directory, true)?;
    let index = pin_directory(directory)?;
    let bucket_path = bucket(directory, &original)?;
    let created = private_stage::create_output_directory(
        &index,
        bucket_path.file_name().ok_or("missing receipt index key")?,
    )
    .ok();
    let bucket_pin = pin_directory(&bucket_path)?;
    let bucket = Bucket {
        path: bucket_path,
        identity: Identity::of(&bucket_pin)?,
    };
    let owns_bucket = match created {
        Some(created) if Identity::of(&created)? == bucket.identity => true,
        Some(_) => return Err("New ZIP candidate index directory changed".into()),
        None => false,
    };
    let stage_name = output
        .stage
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| stage_label(name))
        .ok_or("invalid edit stage name")?;
    let nonce = stage_name.trim_start_matches(".qing-zip-stage-");
    let path = bucket.path.join(nonce).with_extension("dpapi");
    let file = File::options()
        .write(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_POINT)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut lease = Lease {
        file: Some(file),
        bucket,
        owns_bucket,
        #[cfg(test)]
        receipt: path,
    };
    // This handle created the file exclusively, and its pending deletion can
    // make NTFS report zero links. Recheck one live link before any later disposal.
    let written = (|| -> Result<(), String> {
        let length = candidate.metadata().map_err(|e| e.to_string())?.len();
        let file = lease.file.as_mut().expect("new ZIP candidate receipt");
        let record = Record {
            version: 1,
            receipt: Identity::of(file)?,
            bucket: Identity::of(&bucket_pin)?,
            owns_bucket,
            original,
            original_name: source.original_name.clone(),
            parent: Identity::of(&source.parent)?,
            stage_name: stage_name.into(),
            stage: Identity::of(output.stage.root())?,
            output: Identity::of(&output.output_pin)?,
            candidate: Identity::of(candidate)?,
            length,
            digest: digest(candidate, length, control)?,
        };
        let sealed = crate::services::protect(
            &serde_json::to_vec(&record).map_err(|e| e.to_string())?,
            true,
        )?;
        if sealed.len() as u64 > RECORD_LIMIT {
            return Err("ZIP candidate receipt exceeds its size limit".into());
        }
        file.write_all(&sealed)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    })();
    drop(bucket_pin);
    drop(profile);
    written?;
    Ok(lease)
}

/// Consult a single index entry. If the former original is gone, its existing
/// edit journal owns recovery. Nothing in that transaction is disposed here.
pub(crate) fn reclaim_on_open(path: &Path, control: &JobControl) {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        return;
    }
    let Some(directory) = index().filter(|path| path.is_dir()) else {
        return;
    };
    let _ = (|| -> Result<(), String> {
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let original = pin(&path)?;
        reclaim(&directory, &path, &original, control)
    })();
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &[u8] = b"original archive bytes";
    const CANDIDATE: &[u8] = b"completed candidate bytes";

    struct Fixture {
        directory: tempfile::TempDir,
        archive: PathBuf,
        neighbor: PathBuf,
        profile: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let archive = directory.path().join("existing.zip");
            let neighbor = directory.path().join("keep.txt");
            let profile = directory.path().join("my-profile");
            std::fs::write(&archive, ORIGINAL).unwrap();
            std::fs::write(&neighbor, b"user file").unwrap();
            TEST_PROFILE.with(|test_profile| test_profile.replace(Some(profile.clone())));
            Self {
                directory,
                archive,
                neighbor,
                profile,
            }
        }

        fn prepare(&self) -> (EditSource, EditOutput, PathBuf) {
            let source = EditSource::open(&self.archive).unwrap();
            assert!(source.local_ntfs());
            let mut output = source.output().unwrap();
            let candidate = output.disposable_zip_candidate().unwrap();
            candidate.writer().unwrap().write_all(CANDIDATE).unwrap();
            output
                .finish_candidate(&source, candidate, true, &JobControl::new())
                .unwrap();
            let receipt = output
                .candidate_receipt
                .as_ref()
                .expect("confirmed receipt")
                .receipt
                .clone();
            (source, output, receipt)
        }

        fn abandon(output: EditOutput) -> PathBuf {
            // Simulate process termination: handles close without Rust drops
            // applying the normal cleanup policy.
            let EditOutput {
                candidate_receipt,
                output_pin,
                stage,
                path,
            } = output;
            let mut lease = candidate_receipt.unwrap();
            drop(lease.file.take());
            drop(output_pin);
            stage.keep();
            path
        }

        fn assert_untouched(&self) {
            assert_eq!(std::fs::read(&self.archive).unwrap(), ORIGINAL);
            assert_eq!(std::fs::read(&self.neighbor).unwrap(), b"user file");
        }

        fn reclaim(&self) {
            reclaim_on_open(&self.archive, &JobControl::new());
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            TEST_PROFILE.with(|profile| profile.replace(None));
        }
    }

    #[test]
    fn reclaims_only_the_inactive_confirmed_leaf() {
        let fixture = Fixture::new();
        let (source, output, record) = fixture.prepare();
        fixture.reclaim();
        assert!(output.path().is_file());
        assert!(record.is_file());
        let staged = Fixture::abandon(output);
        let another = staged.with_file_name("another.zip");
        std::fs::write(&another, CANDIDATE).unwrap();
        let unrelated = fixture
            .directory
            .path()
            .join(".qing-zip-stage-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join("existing.zip"), b"other operation").unwrap();
        source.reclaim_candidate(&JobControl::new());
        fixture.reclaim();
        assert!(!staged.exists());
        assert!(!record.exists());
        assert!(!record.parent().unwrap().exists());
        assert_eq!(std::fs::read(another).unwrap(), CANDIDATE);
        assert_eq!(
            std::fs::read(unrelated.join("existing.zip")).unwrap(),
            b"other operation"
        );
        fixture.assert_untouched();
    }

    #[test]
    fn stale_receipt_can_retire_after_the_candidate_is_gone() {
        let fixture = Fixture::new();
        let (_source, output, record) = fixture.prepare();
        let staged = Fixture::abandon(output);
        std::fs::remove_file(staged).unwrap();
        fixture.reclaim();
        fixture.reclaim();
        assert!(!record.exists());
        fixture.assert_untouched();
    }

    #[test]
    fn retry_reclaims_prior_candidate_and_keeps_an_active_edit() {
        let fixture = Fixture::new();
        let (_source, abandoned, abandoned_record) = fixture.prepare();
        let old = Fixture::abandon(abandoned);
        let (source, active, live_record) = fixture.prepare();
        source.reclaim_candidate(&JobControl::new());
        assert!(!old.exists());
        assert!(!abandoned_record.exists());
        assert!(live_record.exists());
        assert_eq!(std::fs::read(active.path()).unwrap(), CANDIDATE);
        drop(active);
        assert_eq!(
            std::fs::read_dir(live_record.parent().unwrap())
                .unwrap()
                .count(),
            0
        );
        fixture.assert_untouched();
    }

    #[test]
    fn identical_receipt_recreated_under_the_same_name_cannot_authorize_removal() {
        let fixture = Fixture::new();
        let (_source, output, receipt) = fixture.prepare();
        let staged = Fixture::abandon(output);
        let bytes = std::fs::read(&receipt).unwrap();
        std::fs::rename(&receipt, receipt.with_extension("other")).unwrap();
        std::fs::write(&receipt, &bytes).unwrap();
        fixture.reclaim();
        assert_eq!(std::fs::read(staged).unwrap(), CANDIDATE);
        assert_eq!(std::fs::read(receipt).unwrap(), bytes);
        fixture.assert_untouched();
    }

    #[test]
    fn preserves_candidate_if_its_identity_or_content_changed() {
        for replace in [false, true] {
            let fixture = Fixture::new();
            let (_source, output, record) = fixture.prepare();
            let staged = Fixture::abandon(output);
            let expected = if replace {
                std::fs::rename(&staged, staged.with_file_name("previous.zip")).unwrap();
                CANDIDATE.to_vec()
            } else {
                vec![b'X'; CANDIDATE.len()]
            };
            std::fs::write(&staged, &expected).unwrap();
            fixture.reclaim();
            assert_eq!(std::fs::read(staged).unwrap(), expected);
            assert!(record.is_file());
            fixture.assert_untouched();
        }
    }

    #[test]
    fn preserves_candidate_for_invalid_receipt_or_moved_archive() {
        for moved in [false, true] {
            let fixture = Fixture::new();
            let (source, output, record) = fixture.prepare();
            let staged = Fixture::abandon(output);
            drop(source);
            if moved {
                let moved = fixture.archive.with_file_name("moved.zip");
                std::fs::rename(&fixture.archive, &moved).unwrap();
                fixture.reclaim();
                reclaim_on_open(&moved, &JobControl::new());
                assert_eq!(std::fs::read(moved).unwrap(), ORIGINAL);
            } else {
                std::fs::write(&record, b"unreadable proof").unwrap();
                fixture.reclaim();
                fixture.assert_untouched();
            }
            assert_eq!(std::fs::read(staged).unwrap(), CANDIDATE);
            assert!(record.is_file());
        }
    }

    #[test]
    fn receipt_unavailable_does_not_block_finishing_the_candidate() {
        let fixture = Fixture::new();
        std::fs::write(
            &fixture.profile,
            b"ordinary file instead of a profile directory",
        )
        .unwrap();
        let source = EditSource::open(&fixture.archive).unwrap();
        let mut output = source.output().unwrap();
        let candidate = output.disposable_zip_candidate().unwrap();
        candidate.writer().unwrap().write_all(CANDIDATE).unwrap();
        output
            .finish_candidate(&source, candidate, true, &JobControl::new())
            .unwrap();
        assert!(output.candidate_receipt.is_none());
        assert_eq!(std::fs::read(output.path()).unwrap(), CANDIDATE);
        fixture.assert_untouched();
    }

    #[test]
    fn corrupt_receipt_and_changed_candidate_do_not_block_next_recovery() {
        for corrupt in [false, true] {
            let fixture = Fixture::new();
            let (_source, output, first_record) = fixture.prepare();
            let first_result = Fixture::abandon(output);
            let changed = b"external change";
            std::fs::write(&first_result, changed).unwrap();
            if corrupt {
                std::fs::write(&first_record, b"unreadable proof").unwrap();
            }
            let (_source, output, second_record) = fixture.prepare();
            let second_result = Fixture::abandon(output);
            fixture.reclaim();
            assert_eq!(std::fs::read(first_result).unwrap(), changed);
            assert!(first_record.exists());
            assert!(!second_result.exists());
            assert!(!second_record.exists());
            fixture.assert_untouched();
        }
    }

    #[test]
    fn leaves_the_candidate_with_the_live_commit_journal_until_disarmed() {
        let fixture = Fixture::new();
        let (source, output, receipt) = fixture.prepare();
        let original = id(&source.read).unwrap();
        let proposed = id(&pin(output.path()).unwrap()).unwrap();
        drop(
            journal::Intent::write(
                &output.stage,
                &source.parent,
                &source.path,
                &original,
                &proposed,
                None,
            )
            .unwrap(),
        );
        let intent_path = output.stage.path().join("edit-intent.dpapi");
        let staged = Fixture::abandon(output);
        fixture.reclaim();
        assert!(receipt.exists());
        assert_eq!(std::fs::read(&staged).unwrap(), CANDIDATE);
        // Existing recovery explicitly truncates this receipt once it has
        // returned the original to its public name.
        File::options()
            .write(true)
            .open(&intent_path)
            .unwrap()
            .set_len(0)
            .unwrap();
        fixture.reclaim();
        assert!(!receipt.exists());
        assert!(!staged.exists());
        assert_eq!(std::fs::metadata(intent_path).unwrap().len(), 0);
        fixture.assert_untouched();
    }

    #[test]
    fn removes_empty_index_buckets_after_successful_updates() {
        let fixture = Fixture::new();
        for _ in 0..3 {
            let (source, output, record) = fixture.prepare();
            source.commit(output).unwrap();
            assert_eq!(std::fs::read(&fixture.archive).unwrap(), CANDIDATE);
            assert!(!record.parent().unwrap().exists());
        }
        assert_eq!(
            std::fs::read_dir(fixture.profile.join(DIRECTORY))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(std::fs::read(&fixture.neighbor).unwrap(), b"user file");
    }

    #[test]
    fn existing_journal_restores_claimed_original_before_candidate_lookup() {
        let fixture = Fixture::new();
        let (source, output, receipt) = fixture.prepare();
        let original = id(&source.read).unwrap();
        let proposed = id(&pin(output.path()).unwrap()).unwrap();
        drop(
            journal::Intent::write(
                &output.stage,
                &source.parent,
                &source.path,
                &original,
                &proposed,
                None,
            )
            .unwrap(),
        );
        let claimed = output.stage.path().join("merge");
        std::fs::create_dir(&claimed).unwrap();
        let candidate = Fixture::abandon(output);
        drop(source);
        std::fs::rename(&fixture.archive, claimed.join("existing.zip")).unwrap();
        fixture.reclaim();
        assert!(candidate.exists());
        assert!(receipt.exists());
        assert!(journal::recover_missing(&fixture.archive).unwrap());
        fixture.reclaim();
        assert!(!candidate.exists());
        assert!(!receipt.exists());
        fixture.assert_untouched();
    }
}

pub(super) fn reclaim_on_retry(source: &EditSource, control: &JobControl) {
    let Some(directory) = index().filter(|path| path.is_dir()) else {
        return;
    };
    let _ = reclaim(&directory, &source.path, &source.read, control);
}

fn reclaim(
    directory: &Path,
    path: &Path,
    original: &File,
    control: &JobControl,
) -> Result<(), String> {
    control.check()?;
    if !crate::engine::selected_worker::create_zip::local_ntfs(path, original) {
        return Ok(());
    }
    let identity = Identity::of(original)?;
    let bucket = bucket(directory, &identity)?;
    // Missing receipts are the hot path. Do not enumerate either directory or
    // acquire its ancestry until the one indexed file could contain a record.
    if !bucket.is_dir() {
        return Ok(());
    }
    let profile = directory_guard(&bucket, false)?;
    let directory = Bucket::capture(&bucket)?;
    let mut owns_bucket = false;
    for entry in std::fs::read_dir(&bucket)
        .map_err(|e| e.to_string())?
        .take(RECORDS_PER_LOOKUP)
        .flatten()
    {
        control.check()?;
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".dpapi")) else {
            continue;
        };
        if stem.len() != 32 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        if let Ok(owned) = reclaim_record(
            &entry.path(),
            path,
            original,
            &identity,
            &directory.identity,
            control,
        ) {
            owns_bucket = owned;
            break;
        }
    }
    drop(profile);
    if owns_bucket {
        directory.remove_if_empty();
    }
    Ok(())
}

fn reclaim_record(
    receipt: &Path,
    path: &Path,
    original: &File,
    identity: &Identity,
    bucket: &Identity,
    control: &JobControl,
) -> Result<bool, String> {
    let file = leaf(receipt, GENERIC_READ | DELETE, 0)?;
    private_stage::checked_metadata(&file)?;
    let mut bytes = Vec::new();
    (&file)
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err("ZIP candidate receipt is too large".into());
    }
    let clear = crate::services::protect(&bytes, false)?;
    let record: Record = serde_json::from_slice(&clear).map_err(|e| e.to_string())?;
    if record.version != 1
        || record.receipt != Identity::of(&file)?
        || record.bucket != *bucket
        || record.original != *identity
        || !stage_label(&record.stage_name)
        || receipt.file_stem().and_then(|name| name.to_str())
            != record.stage_name.strip_prefix(".qing-zip-stage-")
        || record.original_name != actual_name(original)?
    {
        return Err("ZIP candidate receipt does not match the public archive".into());
    }
    let parent = path.parent().ok_or("missing archive parent")?;
    let name = path.file_name().ok_or("missing archive filename")?;
    let parent_handle = pin_directory(parent)?;
    if record.parent != Identity::of(&parent_handle)? {
        return Err("ZIP candidate parent changed".into());
    }
    let stage_path = parent.join(&record.stage_name);
    if matches!(std::fs::symlink_metadata(&stage_path), Err(error) if error.kind() == io::ErrorKind::NotFound)
    {
        DisposableZipCandidate::set_disposition(&file, true)?;
        return Ok(record.owns_bucket);
    }
    let stage = pin_directory(&stage_path)?;
    if record.stage != Identity::of(&stage)? {
        return Err("ZIP candidate stage changed".into());
    }
    // The normal transaction takes precedence after it has a durable intent.
    // Its recovery truncates that marker after restoring the public original.
    let _disarmed = journal::lock_disarmed_intent(&stage_path)?;
    let output_path = stage_path.join("output");
    let output = pin_directory(&output_path)?;
    if record.output != Identity::of(&output)? {
        return Err("ZIP candidate directory changed".into());
    }
    let candidate = File::options()
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(1)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(output_path.join(name));
    match candidate {
        Ok(candidate) => {
            let metadata = private_stage::checked_metadata(&candidate)?;
            if !metadata.is_file()
                || metadata.file_attributes() & 0x400 != 0
                || record.candidate != Identity::of(&candidate)?
                || record.length != metadata.len()
                || record.digest != digest(&candidate, record.length, control)?
            {
                return Err("ZIP candidate was replaced or changed".into());
            }
            control.check()?;
            DisposableZipCandidate::set_disposition(&candidate, true)?;
            drop(candidate);
        }
        // Covers interruption both before the first handoff and after removing
        // the exact candidate. Retire only the explicitly authenticated receipt.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    DisposableZipCandidate::set_disposition(&file, true)?;
    Ok(record.owns_bucket)
}
