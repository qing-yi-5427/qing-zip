use super::{id, leaf, pin_directory, publish_with_live_permissions, scratch_path, FileId};
use crate::private_stage::EditPermissions;
use crate::private_stage::PrivateStage;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

const MARKER: &str = "edit-intent.dpapi";
const MAX_RECORD: u64 = 1024 * 1024;
const DOMAIN: &str = "qing-zip/archive-edit-v1";
#[derive(Serialize, Deserialize)]
struct Record {
    domain: String,
    parent: FileId,
    stage: FileId,
    original: FileId,
    proposal: FileId,
    basename: Vec<u16>,
    #[serde(default)]
    explicit_acl: Option<Vec<u32>>,
}

pub(super) struct Intent(File);
impl Intent {
    pub(super) fn write(
        stage: &PrivateStage,
        parent: &File,
        target: &Path,
        original: &FileId,
        proposal: &FileId,
        permissions: Option<&EditPermissions>,
    ) -> Result<Self, String> {
        let basename = target
            .file_name()
            .ok_or("missing archive filename")?
            .encode_wide()
            .collect();
        let record = Record {
            domain: DOMAIN.into(),
            parent: id(parent)?,
            stage: id(stage.root())?,
            original: original.clone(),
            proposal: proposal.clone(),
            basename,
            explicit_acl: permissions.map(EditPermissions::recipe),
        };
        let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
        let sealed = crate::services::protect(&bytes, true)?;
        if sealed.len() as u64 > MAX_RECORD {
            return Err("Edit recovery record is too large".into());
        }
        // An exclusive handle keeps other instances from recovering an active edit.
        let mut marker = File::options()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(stage.path().join(MARKER))
            .map_err(|e| e.to_string())?;
        marker
            .write_all(&sealed)
            .and_then(|_| marker.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(Self(marker))
    }
    pub(super) fn disarm(self) {
        // Disarming must happen while still holding the private file handle.
        let _ = self.0.set_len(0);
    }
}

struct Match {
    original: File,
    marker: File,
    saved: PathBuf,
    previous_parent: File,
    public_proposal: Option<(PathBuf, PathBuf, FileId)>,
    scratch: PathBuf,
    permissions: Option<EditPermissions>,
    ready: bool,
    _pins: Vec<File>,
}

fn match_stage(stage: &Path, parent: &FileId, name: &std::ffi::OsStr) -> Option<Match> {
    let stage_handle = pin_directory(stage).ok()?;
    let mut marker = leaf(&stage.join(MARKER), 0xc0000000, 0).ok()?;
    let len = marker.metadata().ok()?.len();
    if !(1..=MAX_RECORD).contains(&len) {
        return None;
    }
    let mut ciphertext = Vec::new();
    (&mut marker)
        .take(MAX_RECORD + 1)
        .read_to_end(&mut ciphertext)
        .ok()?;
    let record: Record =
        serde_json::from_slice(&crate::services::protect(&ciphertext, false).ok()?).ok()?;
    if record.domain != DOMAIN
        || record.parent != *parent
        || record.stage != id(&stage_handle).ok()?
        || !name.encode_wide().eq(record.basename.iter().copied())
    {
        return None;
    }
    let target_parent = stage.parent()?;
    let target = target_parent.join(name);
    let result_scratch = scratch_path(stage, &target, false).ok()?;
    let original_scratch = scratch_path(stage, &target, true).ok()?;
    let candidates = [
        (stage.join("merge").join(name), true, true),
        (stage.join("original").join(name), true, false),
        (stage.join("output").join(name), false, true),
        (result_scratch.clone(), false, true),
        (original_scratch.clone(), true, false),
    ];
    let mut pins = vec![stage_handle];
    let mut original = None;
    let mut proposal = None;
    let mut public_proposal = None;
    for (path, may_be_original, may_be_proposal) in candidates {
        let Ok(pin) = pin_directory(path.parent()?) else {
            continue;
        };
        let Ok(check) = leaf(&path, 0, 7) else {
            continue;
        };
        let Ok(identity) = id(&check) else { continue };
        if may_be_original && identity == record.original {
            let handle = leaf(&path, 0x10080, 1).ok()?;
            if id(&handle).ok()? != record.original {
                return None;
            }
            original = Some((handle, path, pin));
        } else if may_be_proposal && identity == record.proposal {
            if path == result_scratch
                && matches!(identity, FileId::Extended(..) | FileId::Legacy(..))
            {
                public_proposal = Some((path, stage.join("merge").join(name), identity));
            }
            proposal = Some(check);
            pins.push(pin);
        }
    }
    let ready = proposal.is_some();
    if let Some(guard) = proposal {
        pins.push(guard);
    }
    let (original, saved, previous_parent) = original?;
    let permissions = if let Some(explicit) = record.explicit_acl {
        Some(EditPermissions::from_recipe(explicit).ok()?)
    } else {
        // Compatible with intents written before explicit ACLs were stored.
        EditPermissions::capture(&super::reopen(&original, 0x20000, 7).ok()?).ok()?
    };
    Some(Match {
        original,
        public_proposal,
        marker,
        saved,
        previous_parent,
        scratch: original_scratch,
        permissions,
        ready,
        _pins: pins,
    })
}

fn park_interrupted_proposal(item: &Match) -> Result<(), String> {
    let Some((public, private, expected)) = &item.public_proposal else {
        return Ok(());
    };
    let file = leaf(public, 0x10080, 1)?;
    if id(&file)? != *expected {
        return Ok(());
    } // Never take a same-named external file.
    let parent = pin_directory(private.parent().ok_or("missing private result parent")?)?;
    if let Some(security) = &item.permissions {
        security.protect(&super::reopen(&file, 0x60000, 7)?)?;
    }
    if !super::rename_handle_noclobber_in(&file, private, Some(&parent))? {
        return Err(format!(
            "Uncommitted archive remains at {}",
            public.display()
        ));
    }
    Ok(())
}

pub(in crate::engine) fn recover_missing(input: &Path) -> Result<bool, String> {
    let target = std::path::absolute(input).map_err(|e| e.to_string())?;
    if !matches!(fs::symlink_metadata(&target),Err(ref e) if e.kind()==io::ErrorKind::NotFound) {
        return Ok(false);
    }
    let Some(dir) = target.parent() else {
        return Ok(false);
    };
    let Some(name) = target.file_name() else {
        return Ok(false);
    };
    // Pin every ancestor, so a junction/renamed parent cannot redirect recovery.
    let mut checked = super::CheckedDirectories::default();
    checked.allow_publication(dir)?;
    if !dir.is_dir() {
        return Ok(false);
    }
    checked.check(dir)?;
    let parent = pin_directory(dir)?;
    let parent_id = id(&parent)?;
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let spelling = entry.file_name();
        let Some(nonce) = spelling
            .to_str()
            .and_then(|n| n.strip_prefix(".qing-zip-stage-"))
        else {
            continue;
        };
        if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if let Some(snapshot) = match_stage(&entry.path(), &parent_id, name) {
            found.push(snapshot);
        }
    }
    if found.is_empty() {
        return Ok(false);
    }
    if found.len() == 1 && found[0].ready {
        let item = &found[0];
        let parked = park_interrupted_proposal(item);
        if publish_with_live_permissions(
            &item.original,
            &item.saved,
            &item.previous_parent,
            &target,
            &parent,
            &item.scratch,
            item.permissions.as_ref(),
            || Ok(()),
        )
        .is_ok()
        {
            let _ = found[0].marker.set_len(0);
            parked.map_err(|e| format!("Original archive restored. {e}"))?;
            return Ok(true);
        }
        if fs::symlink_metadata(&target).is_ok() {
            return Ok(false);
        }
    }
    let paths = found
        .iter()
        .map(|m| m.saved.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!("Interrupted archive edit. The original was preserved at:\n{paths}\nTo recover, copy it back only while the requested archive path is empty."))
}
