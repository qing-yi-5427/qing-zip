//! Ownership evidence for rollback of an empty selected-output directory.
//! The bounded handle cache prevents file-ID reuse directly. Beyond the cache
//! NTFS can stamp the created object with a unique change-journal record.
//! FSCTL_READ_FILE_USN_DATA returns the last USN for the handle. A reset can
//! reuse offsets under a new journal ID, so file ID and USN alone do not suffice.
//! See https://learn.microsoft.com/windows/win32/fileio/using-the-change-journal-identifier
use super::{directory_identity, File};
use std::{ffi::c_void, os::windows::io::AsRawHandle};

pub(super) enum Identity {
    Handle(File),
    Owned(File, [u8;32]),
    Journal(Stamp),
}

pub(super) struct Stamp {
    file_id: (u32, u64),
    journal: u64,
    usn: i64,
}

const QUERY_JOURNAL: u32 = 0x900f4;
const WRITE_CLOSE_RECORD: u32 = 0x900ef;
const READ_FILE_USN: u32 = 0x900eb;

#[link(name = "Kernel32")]
extern "system" {
    fn DeviceIoControl(
        file: *mut c_void,
        code: u32,
        input: *const u8,
        input_size: u32,
        output: *mut u8,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

fn query(file: &File, code: u32, output: &mut [u8]) -> Option<usize> {
    let mut written = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            std::ptr::null(),
            0,
            output.as_mut_ptr(),
            output.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    };
    (ok != 0 && (written as usize) <= output.len()).then_some(written as usize)
}

struct Journal {
    id: u64,
    oldest: i64,
    next: i64,
}
impl Journal {
    fn read(file: &File) -> Option<Self> {
        // Query the actual directory's volume, never a guessed drive letter.
        let mut buf = [0; 128];
        if query(file, QUERY_JOURNAL, &mut buf)? < 56 {
            return None;
        }
        let word = |n| i64::from_le_bytes(buf[n..n + 8].try_into().unwrap());
        Some(Self {
            id: u64::from_le_bytes(buf[..8].try_into().unwrap()),
            oldest: word(8).max(word(24)),
            next: word(16),
        })
    }
    fn contains(&self, id: u64, usn: i64) -> bool {
        id != 0 && id == self.id && usn > 0 && usn >= self.oldest && usn < self.next
    }
}

fn file_usn(file: &File, index: u64) -> Option<i64> {
    let mut buf = [0; 1024];
    let len = query(file, READ_FILE_USN, &mut buf)?;
    if len < 60
        || u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize != len
        || u16::from_le_bytes(buf[4..6].try_into().unwrap()) != 2
        || u64::from_le_bytes(buf[8..16].try_into().unwrap()) != index
    {
        return None;
    }
    Some(i64::from_le_bytes(buf[24..32].try_into().unwrap()))
}

pub(super) fn read_revision(file: &File, index: u64) -> Option<(u64, i64)> {
    let before = Journal::read(file)?;
    let usn = file_usn(file, index)?;
    let after = Journal::read(file)?;
    after.contains(before.id, usn).then_some((before.id, usn))
}

impl Stamp {
    pub(super) fn capture(created: &File) -> Option<Self> {
        let before = Journal::read(created)?;
        let file_id = directory_identity(created).ok()?;
        let mut buf = [0; 8];
        if query(created, WRITE_CLOSE_RECORD, &mut buf)? != buf.len() {
            return None;
        }
        let stamp = Self {
            file_id,
            journal: before.id,
            usn: i64::from_le_bytes(buf),
        };
        // Require the exact V2 record to be visible on the creation handle and
        // the same volume journal to span the explicit close-record request.
        stamp.matches(created).then_some(stamp)
    }
    fn matches(&self, current: &File) -> bool {
        if directory_identity(current).ok() != Some(self.file_id) {
            return false;
        }
        if file_usn(current, self.file_id.1) != Some(self.usn) {
            return false;
        }
        // USNs are unique offsets within one journal generation. Deleting and
        // recreating the journal can reuse offsets, so bracket the observation.
        Journal::read(current).is_some_and(|j| j.contains(self.journal, self.usn))
    }
}
impl Identity {
    pub(super) fn owned(file: &File)->Result<Self,String>{
        let handle=file.try_clone().map_err(|e|e.to_string())?;
        let stamp=crate::private_stage::directory_stamp(&handle)?;
        Ok(Self::Owned(handle,stamp))
    }
    pub(super) fn restore_owned_metadata(&mut self,entry:&super::Entry)->Result<(),String>{
        let Self::Owned(file,stamp)=self else {return Err("missing owned directory receipt".into());};
        if crate::private_stage::directory_stamp(file)?!=*stamp {
            return Err("directory was changed externally; metadata preserved".into());
        }
        super::restore_open_file_metadata(file,entry)?;
        *stamp=crate::private_stage::directory_stamp(file)?;
        Ok(())
    }
    pub(super) fn matches(&self, current: &File) -> bool {
        match self {
            Self::Handle(original) => directory_identity(original)
                .ok()
                .is_some_and(|id| directory_identity(current).ok() == Some(id)),
            Self::Owned(original,stamp) => directory_identity(original).ok()
                .is_some_and(|id|directory_identity(current).ok()==Some(id))
                && crate::private_stage::directory_stamp(original).ok()==Some(*stamp),
            Self::Journal(stamp) => stamp.matches(current),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::OpenOptionsExt, path::Path};

    fn open(path: &Path) -> File {
        fs::OpenOptions::new()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(path)
            .unwrap()
    }

    #[test]
    fn expired_or_changed_journals_never_prove_directory_ownership() {
        let journal = Journal {
            id: 17,
            oldest: 20,
            next: 30,
        };
        assert!(journal.contains(17, 20));
        assert!(!journal.contains(18, 20));
        assert!(!journal.contains(17, 19));
        assert!(!journal.contains(17, 30));
        assert!(!journal.contains(0, 21));
    }

    #[test]
    fn recorded_directory_cannot_be_confused_with_its_empty_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("tracked");
        fs::create_dir(&dir).unwrap();
        let Some(mut stamp) = Stamp::capture(&open(&dir)) else {
            // Volumes without a usable NTFS journal leave such directories alone.
            return;
        };
        assert!(stamp.matches(&open(&dir)));
        stamp.journal ^= 1;
        assert!(!stamp.matches(&open(&dir)));
        stamp.journal ^= 1;
        fs::rename(&dir, temp.path().join("moved")).unwrap();
        fs::create_dir(&dir).unwrap();
        assert!(!stamp.matches(&open(&dir)));

        // Even a coincident file ID cannot substitute for the original USN.
        stamp.file_id = directory_identity(&open(&dir)).unwrap();
        assert!(!stamp.matches(&open(&dir)));
    }
}
