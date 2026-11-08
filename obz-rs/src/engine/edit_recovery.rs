use super::{id, leaf, pin_directory, FileId};
use crate::private_stage::PrivateStage;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

const MARKER: &str = "edit-intent.dpapi";
const DOMAIN: &str = "qing-zip/archive-edit-v1";
#[derive(Serialize, Deserialize)]
struct Record {
    domain: String,
    parent: FileId,
    stage: FileId,
    original: FileId,
    proposal: FileId,
    basename: Vec<u16>,
}

pub(super) struct Intent(File);
impl Intent {
    pub(super) fn write(
        stage: &PrivateStage,
        parent: &File,
        target: &Path,
        original: &FileId,
        proposal: &FileId,
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
        };
        let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
        let sealed = crate::services::protect(&bytes, true)?;
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
    ready: bool,
    _pins: Vec<File>,
}

fn match_stage(stage: &Path, parent: &FileId, name: &std::ffi::OsStr) -> Option<Match> {
    let stage_handle = pin_directory(stage).ok()?;
    let mut marker = leaf(&stage.join(MARKER), 0xc0000000, 0).ok()?;
    let len = marker.metadata().ok()?.len();
    if !(1..=16384).contains(&len) {
        return None;
    }
    let mut ciphertext = Vec::new();
    (&mut marker)
        .take(16385)
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
    let mut pins = vec![stage_handle];
    let mut original = None;
    let mut proposal = None;
    for dir in ["merge", "original", "output"] {
        let directory = stage.join(dir);
        let Ok(pin) = pin_directory(&directory) else {
            continue;
        };
        pins.push(pin);
        let path = directory.join(name);
        let Ok(check) = leaf(&path, 0, 7) else {
            continue;
        };
        let Ok(identity) = id(&check) else { continue };
        if dir != "output" && identity == record.original {
            // Lock the exact original against writes/renames before offering it.
            let handle = leaf(&path, 0x10080, 1).ok()?;
            if id(&handle).ok()? != record.original {
                return None;
            }
            original = Some((handle, path));
        } else if dir != "original" && identity == record.proposal {
            // Once a proposal has been published its private name is gone. Do
            // not resurrect an archive the user deliberately removed later.
            proposal = Some(check);
        }
    }
    let ready = proposal.is_some();
    if let Some(guard) = proposal {
        pins.push(guard);
    }
    let (original, saved) = original?;
    Some(Match {
        original,
        marker,
        saved,
        ready,
        _pins: pins,
    })
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
        if super::rename_handle_noclobber_in(&found[0].original, &target, Some(&parent))? {
            let _ = found[0].marker.set_len(0);
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
