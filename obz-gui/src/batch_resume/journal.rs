use super::*;

const VERSION: u32 = 2;
pub(super) const MARKER: &[u8] = br#"{"version":2,"#;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finished {
    status: Status,
    has_output: bool,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Admission {
    index: usize,
    previous: Option<Finished>,
}

pub(super) struct Journal {
    root: PathBuf,
    writer: Option<File>,
    _lease: File,
    plan: BatchPlan,
    last: Option<usize>,
}

fn next_pending(plan: &BatchPlan, last: Option<usize>) -> Option<usize> {
    plan.entries
        .iter()
        .enumerate()
        .skip(last.map_or(0, |last| last + 1))
        .find_map(|(index, entry)| (entry.status == Status::Pending).then_some(index))
}

fn validate(plan: &BatchPlan, last: Option<usize>, admission: &Admission) -> bool {
    next_pending(plan, last) == Some(admission.index)
        && match (last, admission.previous) {
            (Some(_), Some(finished)) => finished.status != Status::Pending,
            (None, None) => true,
            _ => false,
        }
}

fn apply(plan: &mut BatchPlan, last: &mut Option<usize>, admission: Admission) {
    if let (Some(previous), Some(finished)) = (*last, admission.previous) {
        plan.entries[previous].status = finished.status;
        plan.entries[previous].has_output = finished.has_output;
    }
    // Admission proves the archive may have reached the engine. Until a later
    // record establishes an outcome it must never be presented as unstarted.
    plan.entries[admission.index].status = Status::Interrupted;
    *last = Some(admission.index);
}

pub(super) fn decode(bytes: &[u8]) -> Option<SavedBatch> {
    let mut lines = bytes.strip_suffix(b"\n")?.split(|byte| *byte == b'\n');
    let mut record: SavedBatch = serde_json::from_slice(lines.next()?).ok()?;
    if record.version != VERSION
        || record.revision.is_empty()
        || !Path::new(&record.plan.destination).is_absolute()
        || record.plan.entries.iter().any(|entry| {
            !Path::new(&entry.path).is_absolute() || !Path::new(&entry.dest).is_absolute()
        })
    {
        return None;
    }
    let mut last = None;
    for line in lines {
        let admission = serde_json::from_slice(line).ok()?;
        if !validate(&record.plan, last, &admission) {
            return None;
        }
        apply(&mut record.plan, &mut last, admission);
    }
    Some(record)
}

impl Journal {
    pub(super) fn begin(
        root: &Path,
        plan: Option<BatchPlan>,
        revision: Option<&str>,
    ) -> Result<Option<(Self, SavedBatch)>, String> {
        let lease = lock(root)?;
        let plan = match (plan, revision, read(root)?) {
            (Some(plan), None, Record::Missing | Record::Finished) => plan.normalize()?,
            (None, Some(revision), Record::Ready(record) | Record::Running(record))
                if record.revision == revision =>
            {
                record.plan
            }
            (
                None,
                Some(_),
                Record::Missing | Record::Finished | Record::Ready(_) | Record::Running(_),
            ) => return Ok(None),
            _ => return Err("当前待继续记录已存在或无法使用。".into()),
        };
        let record = SavedBatch {
            version: VERSION,
            revision: new_revision()?,
            plan,
        };
        let writer = persist(root, &record, true)?;
        let active = Self {
            root: root.to_path_buf(),
            _lease: lease,
            writer: Some(writer),
            plan: record.plan.clone(),
            last: None,
        };
        Ok(Some((active, record)))
    }

    pub(super) fn admit(&mut self, admission: Admission) -> Result<(), String> {
        if !validate(&self.plan, self.last, &admission) {
            return Err("批量项目顺序与续跑记录不一致。".into());
        }
        let mut encoded = serde_json::to_vec(&admission).map_err(|e| e.to_string())?;
        encoded.push(b'\n');
        let writer = self.writer.as_mut().ok_or("当前批量续跑记录已停止。")?;
        writer
            .write_all(&encoded)
            .and_then(|_| writer.sync_data())
            .map_err(|e| e.to_string())?;
        apply(&mut self.plan, &mut self.last, admission);
        Ok(())
    }

    pub(super) fn seal(&mut self, plan: BatchPlan) -> Result<SavedBatch, String> {
        // Windows replacement requires releasing this file handle, but the
        // record lease still excludes another instance until the swap ends.
        self.writer.take();
        save_stopped(&self.root, plan)
    }

    pub(super) fn finish(mut self) {
        self.writer.take();
        if next_pending(&self.plan, self.last).is_none() {
            let _ = fs::remove_file(self.root.join(RECORD));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_admission_write_preserves_the_unstarted_entries() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let plan: BatchPlan = serde_json::from_value(serde_json::json!({
            "destination": "out",
            "overwrite": "overwrite",
            "entries": [
                {"path":"first.zip","dest":"out/1-first","status":"pending","hasOutput":false},
                {"path":"second.zip","dest":"out/2-second","status":"pending","hasOutput":false},
            ],
        }))
        .unwrap();
        let (mut journal, _) = Journal::begin(root, Some(plan), None).unwrap().unwrap();
        let path = root.join(RECORD);
        let before = fs::read(&path).unwrap();
        journal.writer = Some(File::open(&path).unwrap());
        assert!(journal
            .admit(Admission {
                index: 0,
                previous: None
            })
            .is_err());
        assert!(journal
            .plan
            .entries
            .iter()
            .all(|entry| entry.status == Status::Pending));
        drop(journal);
        assert_eq!(fs::read(&path).unwrap(), before);
        let BatchResumeState::Ready { record } = load(root).unwrap() else {
            panic!("missing log");
        };
        assert!(record
            .plan
            .entries
            .iter()
            .all(|entry| entry.status == Status::Pending));
    }
}
