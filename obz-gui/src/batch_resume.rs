//! A queue for explicit continuation after stopping or exiting.
mod journal;

use journal::{Admission, Journal};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, ErrorKind, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

const RECORD: &str = "pending.json";
const VERSION: u32 = 1;

// Individual extraction jobs may all be idle while the main window hands off
// to the next entry or writes its paused queue.
static RUNNING: AtomicBool = AtomicBool::new(false);
static JOURNAL: Mutex<Option<Journal>> = Mutex::new(None);

pub(crate) fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

#[tauri::command]
pub(crate) fn set_batch_resume_running(active: bool, active_revision: Option<String>) {
    if !active {
        let mut slot = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        if slot
            .as_ref()
            .is_some_and(|journal| !journal.owns(active_revision.as_deref()))
        {
            return;
        }
        if let Some(journal) = slot.take() {
            journal.finish();
        }
    }
    RUNNING.store(active, Ordering::SeqCst);
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Status {
    Complete,
    Partial,
    Failed,
    Cancelled,
    Interrupted,
    Pending,
}

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Overwrite {
    Error,
    Rename,
    Skip,
    Overwrite,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    path: String,
    dest: String,
    status: Status,
    has_output: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct BatchPlan {
    destination: String,
    overwrite: Overwrite,
    entries: Vec<Entry>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct SavedBatch {
    version: u32,
    revision: String,
    plan: BatchPlan,
}

fn root() -> Result<PathBuf, String> {
    let path = std::env::var_os("LOCALAPPDATA").ok_or("找不到本地配置目录。")?;
    Ok(PathBuf::from(path).join("qing-zip").join("BatchResume"))
}

fn absolute(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("续跑记录包含无效路径。".into());
    }
    std::path::absolute(path)
        .map_err(|e| e.to_string())?
        .into_os_string()
        .into_string()
        .map_err(|_| "续跑记录包含无效路径。".into())
}

impl BatchPlan {
    fn normalize(mut self) -> Result<Self, String> {
        if !self
            .entries
            .iter()
            .any(|entry| entry.status == Status::Pending)
        {
            return Err("没有未开始的批量解压项目。".into());
        }
        self.destination = absolute(&self.destination)?;
        for entry in &mut self.entries {
            entry.path = absolute(&entry.path)?;
            entry.dest = absolute(&entry.dest)?;
        }
        Ok(self)
    }
}

enum Record {
    Missing,
    Ready(SavedBatch),
    Running(SavedBatch),

    Damaged(Vec<u8>),
    Unsupported,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum BatchResumeState {
    Empty,
    Ready { record: SavedBatch },
    Damaged,
    Unsupported,
}

fn read(root: &Path) -> Result<Record, String> {
    let path = root.join(RECORD);
    crate::desktop::no_links(&path)?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Record::Missing),
        Err(error) => return Err(error.to_string()),
    };
    if bytes.starts_with(journal::MARKER) {
        return Ok(match journal::decode(&bytes) {
            Some(record) => Record::Running(record),
            None => Record::Damaged(bytes),
        });
    }
    let document = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(document) => document,
        // Current records always begin with this marker, even when a write was cut short.
        Err(_) if bytes.is_empty() || bytes.starts_with(br#"{"version":1,"#) => {
            return Ok(Record::Damaged(bytes))
        }
        Err(_) => return Ok(Record::Unsupported),
    };
    if document.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(VERSION)) {
        return Ok(Record::Unsupported);
    }
    let Ok(record) = serde_json::from_value::<SavedBatch>(document) else {
        return Ok(Record::Damaged(bytes));
    };
    if record.revision.is_empty()
        || !Path::new(&record.plan.destination).is_absolute()
        || !record
            .plan
            .entries
            .iter()
            .any(|entry| entry.status == Status::Pending)
        || record.plan.entries.iter().any(|entry| {
            !Path::new(&entry.path).is_absolute() || !Path::new(&entry.dest).is_absolute()
        })
    {
        return Ok(Record::Damaged(bytes));
    }
    Ok(Record::Ready(record))
}

fn lock(root: &Path) -> Result<File, String> {
    let path = root.join("record.lock");
    crate::desktop::no_links(&path)?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    // Hold an exclusive Windows file handle for the complete compare-and-update.
    // The lock file remains in place, so another process cannot acquire a second lock object.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .map_err(|e| e.to_string())
}

fn new_revision() -> Result<String, String> {
    let id = unsafe { windows::Win32::System::Com::CoCreateGuid() }.map_err(|e| e.to_string())?;
    Ok(format!("{id:?}"))
}

fn persist(root: &Path, record: &SavedBatch, journal: bool) -> Result<File, String> {
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
    {
        let mut writer = BufWriter::new(file.as_file_mut());
        serde_json::to_writer(&mut writer, record).map_err(|e| e.to_string())?;
        if journal {
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(root.join(RECORD)).map_err(|e| e.to_string())
}

fn save_stopped(root: &Path, plan: BatchPlan) -> Result<SavedBatch, String> {
    let record = SavedBatch {
        version: VERSION,
        revision: new_revision()?,
        plan,
    };
    persist(root, &record, false)?;
    Ok(record)
}

fn save(root: &Path, plan: BatchPlan, previous: Option<&str>) -> Result<SavedBatch, String> {
    let plan = plan.normalize()?;
    let _lock = lock(root)?;
    match read(root)? {
        Record::Missing if previous.is_none() => {}
        Record::Ready(record) if previous == Some(record.revision.as_str()) => {}
        _ => return Err("当前待继续记录无法更新。".into()),
    }
    save_stopped(root, plan)
}

fn take(root: &Path, revision: &str) -> Result<Option<SavedBatch>, String> {
    let _lock = lock(root)?;
    let record = match read(root)? {
        Record::Missing => return Ok(None),
        Record::Ready(record) | Record::Running(record) => record,
        _ => return Err("当前待继续记录无法读取。".into()),
    };
    if record.revision != revision {
        return Ok(None);
    }
    fs::remove_file(root.join(RECORD)).map_err(|e| e.to_string())?;
    Ok(Some(record))
}

fn backup_damaged(root: &Path) -> Result<PathBuf, String> {
    let _lock = lock(root)?;
    let Record::Damaged(bytes) = read(root)? else {
        return Err("没有可以备份的损坏记录。".into());
    };
    let mut file = tempfile::Builder::new()
        .prefix("damaged-")
        .suffix(".json")
        .tempfile_in(root)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    let (_, backup) = file.keep().map_err(|e| e.to_string())?;
    fs::remove_file(root.join(RECORD)).map_err(|e| e.to_string())?;
    Ok(backup)
}

fn load(root: &Path) -> Result<BatchResumeState, String> {
    let record = read(root)?;
    // Existing v1 records are readable during a short compare-and-swap.
    // A v2 record is offered only after the running GUI releases its lease.
    let record = if matches!(record, Record::Running(_) | Record::Damaged(_)) {
        let Ok(_guard) = lock(root) else {
            return Ok(BatchResumeState::Empty);
        };
        read(root)?
    } else {
        record
    };
    Ok(match record {
        Record::Missing => BatchResumeState::Empty,
        Record::Ready(record) | Record::Running(record) => BatchResumeState::Ready { record },
        Record::Damaged(_) => BatchResumeState::Damaged,
        Record::Unsupported => BatchResumeState::Unsupported,
    })
}

#[tauri::command]
pub(crate) async fn load_batch_resume() -> Result<BatchResumeState, String> {
    tauri::async_runtime::spawn_blocking(|| load(&root()?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn begin_batch_resume_run(
    plan: Option<BatchPlan>,
    revision: Option<String>,
) -> Result<Option<SavedBatch>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut active = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            return Err("当前批量任务已有续跑记录。".into());
        }
        let Some((journal, record)) = Journal::begin(&root()?, plan, revision.as_deref())? else {
            return Ok(None);
        };
        *active = Some(journal);
        Ok(Some(record))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn admit_batch_archive(
    active_revision: String,
    admission: Admission,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        JOURNAL
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_mut()
            .filter(|journal| journal.owns(Some(&active_revision)))
            .ok_or_else(|| "当前批量任务的续跑记录已关闭。".to_string())?
            .admit(admission)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn backup_damaged_batch_resume() -> Result<PathBuf, String> {
    tauri::async_runtime::spawn_blocking(|| backup_damaged(&root()?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn save_batch_resume(
    plan: BatchPlan,
    previous: Option<String>,
    active_revision: Option<String>,
) -> Result<SavedBatch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut active = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(journal) = active.as_mut() {
            if previous.is_some() || !journal.owns(active_revision.as_deref()) {
                return Err("当前待继续记录已变更。".into());
            }
            let record = journal.seal(plan.normalize()?)?;
            active.take();
            Ok(record)
        } else if active_revision.is_none() {
            save(&root()?, plan, previous.as_deref())
        } else {
            Err("当前批量续跑记录已结束。".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn take_batch_resume(revision: String) -> Result<Option<SavedBatch>, String> {
    tauri::async_runtime::spawn_blocking(move || take(&root()?, &revision))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> BatchPlan {
        serde_json::from_value(serde_json::json!({
            "destination":"out",
            "overwrite":"rename",
            "password":"never written",
            "entries":[
                {"path":"a.zip","dest":"out/1-a","status":"complete","hasOutput":true,"message":"never written"},
                {"path":"b.zip","dest":"out/2-b","status":"interrupted","hasOutput":false},
                {"path":"c.zip","dest":"out/3-c","status":"pending","hasOutput":false}
            ]
        })).unwrap()
    }

    #[test]
    fn sealed_batch_normalizes_paths_and_can_be_consumed_once() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(read(root.path()).unwrap(), Record::Missing));
        let saved = save(root.path(), plan(), None).unwrap();
        let disk = fs::read_to_string(root.path().join(RECORD)).unwrap();
        assert!(!disk.contains("never written") && !disk.contains("password"));
        let first = take(root.path(), &saved.revision).unwrap().unwrap();
        assert!(Path::new(&first.plan.destination).is_absolute());
        assert!(Path::new(&first.plan.entries[2].dest).ends_with("out/3-c"));
        assert!(first.plan.entries[0].status == Status::Complete);
        assert!(first.plan.entries[1].status == Status::Interrupted);
        assert!(take(root.path(), &saved.revision).unwrap().is_none());
    }

    #[test]
    fn competing_or_stale_windows_cannot_take_or_replace_another_record() {
        let root = tempfile::tempdir().unwrap();
        let saved = save(root.path(), plan(), None).unwrap();
        let held = lock(root.path()).unwrap();
        assert!(take(root.path(), &saved.revision).is_err());
        assert!(save(root.path(), plan(), Some(&saved.revision)).is_err());
        drop(held);
        let fresh = save(root.path(), plan(), Some(&saved.revision)).unwrap();
        assert!(take(root.path(), &saved.revision).unwrap().is_none());
        assert!(save(root.path(), plan(), Some(&saved.revision)).is_err());
        assert!(take(root.path(), &fresh.revision).unwrap().is_some());
    }

    #[test]
    fn truncated_current_record_is_backed_up_before_a_new_record_can_be_saved() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join(RECORD);
        let damaged = br#"{"version":1,"revision":"unfinished","#;
        fs::write(&record, damaged).unwrap();
        assert!(matches!(read(root.path()).unwrap(), Record::Damaged(_)));
        assert!(save(root.path(), plan(), None).is_err());
        assert_eq!(fs::read(&record).unwrap(), damaged);
        let backup = backup_damaged(root.path()).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), damaged);
        assert!(!record.exists());
        let new = save(root.path(), plan(), None).unwrap();
        assert!(take(root.path(), &new.revision).unwrap().is_some());
        assert_eq!(fs::read(&backup).unwrap(), damaged);
    }

    #[test]
    fn a_newer_or_unidentified_record_is_never_backed_up_or_replaced() {
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join(RECORD);
        for bytes in [
            br#"{"version":3,"plan":{"future":true}}"#.as_slice(),
            br#"{"version":3,"#,
        ] {
            fs::write(&record, bytes).unwrap();
            assert!(matches!(read(root.path()).unwrap(), Record::Unsupported));
            assert!(backup_damaged(root.path()).is_err());
            assert!(save(root.path(), plan(), None).is_err());
            assert!(take(root.path(), "old").is_err());
            assert_eq!(fs::read(&record).unwrap(), bytes);
        }
    }
    fn fresh_plan() -> BatchPlan {
        let mut plan = plan();
        for entry in &mut plan.entries {
            entry.status = Status::Pending;
            entry.has_output = false;
        }
        plan
    }

    fn next(index: usize, previous: Option<&str>) -> Admission {
        serde_json::from_value(serde_json::json!({
            "index": index,
            "previous": previous.map(|status| serde_json::json!({ "status": status, "hasOutput": true })),
        })).unwrap()
    }

    #[test]
    fn a_large_plan_remains_complete_after_registering_the_first_archive() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let mut plan = fresh_plan();
        let first = plan.entries[0].clone();
        plan.entries = (0..512)
            .map(|i| {
                let mut entry = first.clone();
                entry.path = root.join(format!("archive-{i}.zip")).display().to_string();
                entry
            })
            .collect();
        let expected = plan.entries.last().unwrap().path.clone();
        let (mut journal, _) = Journal::begin(root, Some(plan), None).unwrap().unwrap();
        journal.admit(next(0, None)).unwrap();
        drop(journal);
        let BatchResumeState::Ready { record } = load(root).unwrap() else {
            panic!("expected a complete persisted plan");
        };
        assert_eq!(record.plan.entries.len(), 512);
        assert!(record.plan.entries[0].status == Status::Interrupted);
        assert!(record.plan.entries[1..]
            .iter()
            .all(|e| e.status == Status::Pending));
        assert_eq!(record.plan.entries.last().unwrap().path, expected);
    }

    #[test]
    fn crash_record_hides_live_jobs_and_only_preserves_unstarted_tail() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let (mut journal, first) = Journal::begin(root, Some(fresh_plan()), None)
            .unwrap()
            .unwrap();
        journal.admit(next(0, None)).unwrap();
        journal.admit(next(1, Some("complete"))).unwrap();
        assert!(matches!(load(root).unwrap(), BatchResumeState::Empty));
        assert!(take(root, &first.revision).is_err());
        drop(journal); // This models the OS closing a crashed process's handles.
        let BatchResumeState::Ready { record } = load(root).unwrap() else {
            panic!("missing tail");
        };
        assert_eq!(record.version, 2);
        assert!(record.plan.entries[0].status == Status::Complete);
        assert!(record.plan.entries[0].has_output);
        assert!(record.plan.entries[1].status == Status::Interrupted);
        assert!(record.plan.entries[2].status == Status::Pending);
        let (mut resumed, _) = Journal::begin(root, None, Some(&record.revision))
            .unwrap()
            .unwrap();
        assert!(resumed.admit(next(1, None)).is_err());
        resumed.admit(next(2, None)).unwrap();
        resumed.finish();
        assert!(matches!(load(root).unwrap(), BatchResumeState::Empty));
    }

    #[test]
    fn explicit_resume_swaps_the_old_pause_record_atomically_and_preserves_the_tail() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let paused = save(root, plan(), None).unwrap();
        assert!(Journal::begin(root, Some(fresh_plan()), None).is_err());
        assert!(Journal::begin(root, None, Some("stale")).unwrap().is_none());
        let (journal, active) = Journal::begin(root, None, Some(&paused.revision))
            .unwrap()
            .unwrap();
        assert_ne!(paused.revision, active.revision);
        drop(journal); // Exit before the first dispatch.
        let BatchResumeState::Ready { record } = load(root).unwrap() else {
            panic!("missing tail");
        };
        assert!(record.plan.entries[0].status == Status::Complete);
        assert!(record.plan.entries[1].status == Status::Interrupted);
        assert!(record.plan.entries[2].status == Status::Pending);
        assert!(take(root, &paused.revision).unwrap().is_none());
        assert!(take(root, &active.revision).unwrap().is_some());
    }

    #[test]
    fn incomplete_admission_record_is_backed_up_instead_of_replayed() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let (journal, _) = Journal::begin(root, Some(fresh_plan()), None)
            .unwrap()
            .unwrap();
        drop(journal);
        let mut file = OpenOptions::new()
            .append(true)
            .open(root.join(RECORD))
            .unwrap();
        file.write_all(br#"{"index":0,"#).unwrap();
        drop(file);
        assert!(matches!(load(root).unwrap(), BatchResumeState::Damaged));
        let before = fs::read(root.join(RECORD)).unwrap();
        assert_eq!(fs::read(backup_damaged(root).unwrap()).unwrap(), before);
    }
    #[test]
    fn ongoing_journal_becomes_the_existing_stopped_schema_without_a_lock_gap() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let (mut journal, _) = Journal::begin(root, Some(fresh_plan()), None)
            .unwrap()
            .unwrap();
        journal.admit(next(0, None)).unwrap();
        let mut different = plan().normalize().unwrap();
        different.entries[0].dest.push_str("-other");
        assert!(journal.seal(different).is_err());
        let stopped = journal.seal(plan().normalize().unwrap()).unwrap();
        assert!(take(root, &stopped.revision).is_err());
        drop(journal);
        let resumed = take(root, &stopped.revision).unwrap().unwrap();
        assert_eq!(resumed.version, 1);
        assert!(resumed.plan.entries[2].status == Status::Pending);
    }
    #[test]
    fn crashing_after_the_last_admission_retains_an_inspectable_record() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let (mut journal, _) = Journal::begin(root, Some(plan()), None).unwrap().unwrap();
        journal.admit(next(2, None)).unwrap();
        drop(journal);
        let BatchResumeState::Ready { record } = load(root).unwrap() else {
            panic!("missing inspection record");
        };
        assert!(record
            .plan
            .entries
            .iter()
            .all(|entry| entry.status != Status::Pending));
        assert!(record.plan.entries[2].status == Status::Interrupted);
        assert!(Journal::begin(root, None, Some(&record.revision))
            .unwrap()
            .is_none());
        assert!(take(root, &record.revision).unwrap().is_some());
    }
}
