//! Compatibility for old-numbered RAR members stored by unpack version 15.
use super::{
    create_staged_output, validate_name, CheckedDirectories, EngineOptions, Entry, JobControl,
};
use crate::engine_stream::PinnedSource;
use rars::{rar15_40::Block, ArchiveReadOptions, ArchiveReader};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

const SIGNATURE: &[u8] = b"Rar!\x1a\x07\0";
const MAIN_SIZE: usize = 13;
const FIRST_HEADER: usize = SIGNATURE.len() + MAIN_SIZE;
const VOLUME: u16 = 0x0001;
const FIRST_VOLUME: u16 = 0x0100;
const SPLIT_FLAGS: u16 = 0x0003;
const LONG_BLOCK: u16 = 0x8000;
const STORED_VERSION: u8 = 15;
const STORED_METHOD: u8 = 0x30;
const MAX_HEADERS: u64 = 65_536;
const MAX_METADATA: u64 = 16 * 1024 * 1024;

struct HeaderBudget {
    count: u64,
    bytes: u64,
}

impl HeaderBudget {
    fn options(&self) -> ArchiveReadOptions<'static> {
        ArchiveReadOptions::default()
            .with_max_header_count(self.count)
            .with_max_header_bytes(self.bytes)
    }

    fn charge(&mut self, bytes: u64) -> Result<(), String> {
        self.count = self
            .count
            .checked_sub(1)
            .ok_or("classic RAR header count limit")?;
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or("classic RAR header bytes limit")?;
        Ok(())
    }
}

struct Fragment {
    volume: usize,
    start: u64,
    size: u64,
    crc: u32,
}

struct Member {
    name: String,
    size: u64,
    crc: u32,
    fragments: Vec<Fragment>,
}

pub(super) struct Plan {
    // Keep every source pinned through writing and publication.
    later: Vec<PinnedSource>,
    members: Vec<Member>,
}

struct ControlledFile {
    file: File,
    control: JobControl,
}

impl Read for ControlledFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.control.check().map_err(io::Error::other)?;
        self.file.read(buf)
    }
}

impl Seek for ControlledFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.control.check().map_err(io::Error::other)?;
        self.file.seek(position)
    }
}

fn candidate(file: &mut File) -> Result<bool, String> {
    // Gate the parser on a canonical initial volume with a version-15 stored file.
    let mut header = [0; FIRST_HEADER + 26];
    file.rewind().map_err(|e| e.to_string())?;
    let read = file.read_exact(&mut header);
    file.rewind().map_err(|e| e.to_string())?;
    match read {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(error) => return Err(error.to_string()),
    }
    let main = &header[SIGNATURE.len()..FIRST_HEADER];
    let first = &header[FIRST_HEADER..];
    let flags = u16::from_le_bytes([main[3], main[4]]);
    Ok(header.starts_with(SIGNATURE)
        && main[2] == 0x73
        && flags & VOLUME != 0
        && flags & !(VOLUME | FIRST_VOLUME) == 0
        && main[5..7] == (MAIN_SIZE as u16).to_le_bytes()
        && first[2] == 0x74
        && first[24..26] == [STORED_VERSION, STORED_METHOD])
}

impl Plan {
    pub(super) fn prepare(
        path: &Path,
        source: &mut File,
        opts: &EngineOptions,
        ctl: &JobControl,
    ) -> Result<Option<Self>, String> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rar"))
            || opts.codepage.is_some()
            || opts.password.is_some()
            || crate::machine_policy::scope().restricted()
            || !candidate(source)?
            || !super::using_process_token()
        {
            return Ok(None);
        }
        let Some((4, mut siblings)) =
            super::selected_worker::rar_listing::direct_volumes(path, source, ctl)?
        else {
            return Ok(None);
        };
        siblings.sort_unstable_by_key(|(name, _)| name.to_ascii_lowercase());
        let mut later = Vec::new();
        for (index, (name, pin)) in siblings.into_iter().enumerate() {
            // Match the decoder's old .r00 through .z99 sequence and reject gaps.
            if index >= 900 {
                return Ok(None);
            }
            let extension = format!(
                "{}{:02}",
                char::from(b'r' + (index / 100) as u8),
                index % 100
            );
            let expected = path.with_extension(extension);
            if !expected
                .file_name()
                .and_then(|leaf| leaf.to_str())
                .is_some_and(|leaf| leaf.eq_ignore_ascii_case(&name))
            {
                return Ok(None);
            }
            later.push(pin);
        }
        if later.is_empty() {
            return Ok(None);
        }

        let mut members = Vec::new();
        let mut current: Option<Member> = None;
        let mut repeated_full_crc = false;
        let mut budget = HeaderBudget {
            count: MAX_HEADERS,
            bytes: MAX_METADATA,
        };
        for (index, volume) in std::iter::once(&*source)
            .chain(later.iter().map(|pin| &**pin))
            .enumerate()
        {
            ctl.check()?;
            let file = ControlledFile {
                file: volume.try_clone().map_err(|e| e.to_string())?,
                control: ctl.clone(),
            };
            let parsed = match ArchiveReader::read_reader_with_options(file, budget.options()) {
                Ok(parsed) => parsed,
                Err(error)
                    if matches!(
                        error.root_cause(),
                        rars::Error::HeaderCountLimitExceeded { .. }
                            | rars::Error::HeaderBytesLimitExceeded { .. }
                    ) =>
                {
                    return Ok(None)
                }
                Err(error) if matches!(error.root_cause(), rars::Error::CrcMismatch { .. }) => {
                    return Err(format!("classic RAR header CRC mismatch: {error}"));
                }
                Err(error) => return Err(error.to_string()),
            };
            let Some(archive) = parsed.as_rar15_40() else {
                return Ok(None);
            };
            let expected_flags = if index == 0 {
                VOLUME | (archive.main.flags & FIRST_VOLUME)
            } else {
                VOLUME
            };
            if archive.sfx_offset != 0
                || usize::from(archive.main.head_size) != MAIN_SIZE
                || archive.main.flags != expected_flags
                || archive.blocks.is_empty()
            {
                return Ok(None);
            }
            budget.charge(MAIN_SIZE as u64)?;
            let mut position = FIRST_HEADER as u64;
            for (ordinal, block) in archive.blocks.iter().enumerate() {
                ctl.check()?;
                let Block::File(file) = block else {
                    return Ok(None);
                };
                if file.unp_ver != STORED_VERSION
                    || !file.is_stored()
                    || file.is_directory()
                    || file.block.flags & !SPLIT_FLAGS != LONG_BLOCK
                    || !file.name.is_ascii()
                    || file.block.offset as u64 != position
                    || file.is_split_before() && ordinal != 0
                    || file.is_split_after() && ordinal + 1 != archive.blocks.len()
                {
                    return Ok(None);
                }
                budget.charge(u64::from(file.block.head_size))?;
                let raw = std::str::from_utf8(&file.name).map_err(|e| e.to_string())?;
                let name = raw.replace('\\', "/");
                let start = file.packed_range.start as u64;
                let end = file.packed_range.end as u64;
                if start != position + u64::from(file.block.head_size)
                    || end.checked_sub(start) != Some(file.pack_size)
                {
                    return Err("invalid classic RAR stored extent".into());
                }
                position = end;
                let fragment = Fragment {
                    volume: index,
                    start,
                    size: file.pack_size,
                    crc: file.file_crc,
                };
                let mut member = match current.take() {
                    Some(member)
                        if file.is_split_before()
                            && member.name == name
                            && member.size == file.unp_size =>
                    {
                        member
                    }
                    None if !file.is_split_before() => Member {
                        name,
                        size: file.unp_size,
                        crc: file.file_crc,
                        fragments: Vec::new(),
                    },
                    _ => {
                        return Err("incomplete or inconsistent classic RAR volume sequence".into())
                    }
                };
                member.fragments.push(fragment);
                if file.is_split_after() {
                    current = Some(member);
                } else {
                    member.crc = file.file_crc;
                    repeated_full_crc |= member.fragments[..member.fragments.len() - 1]
                        .iter()
                        .any(|part| part.crc == member.crc);
                    members.push(member);
                }
            }
            if position != volume.metadata().map_err(|e| e.to_string())?.len()
                || current.is_none() && index < later.len()
            {
                return Ok(None);
            }
        }
        if current.is_some() {
            return Err("RAR archive is missing a required volume".into());
        }
        Ok(repeated_full_crc.then_some(Self { later, members }))
    }

    pub(super) fn verify_manifest(&self, entries: &[Entry]) -> Result<(), String> {
        let by_name: HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.name.replace('\\', "/"), entry))
            .collect();
        if self.members.len() != entries.len() || by_name.len() != entries.len() {
            return Err("classic RAR manifest count mismatch".into());
        }
        let mut seen = HashSet::new();
        for member in &self.members {
            validate_name(&member.name)?;
            if !seen.insert(&member.name)
                || !by_name.get(&member.name).is_some_and(|entry| {
                    !entry.is_dir
                        && !entry.encrypted
                        && entry.size_known
                        && entry.size == member.size
                        && entry.packed_size == member.size
                        && u32::from_str_radix(&entry.crc, 16).ok() == Some(member.crc)
                })
            {
                return Err("classic RAR manifest disagrees with the archive directory".into());
            }
        }
        Ok(())
    }

    pub(super) fn stage(
        &self,
        source: &File,
        selected: &[&Entry],
        stage: &Path,
        guards: &mut CheckedDirectories,
        ctl: &JobControl,
    ) -> Result<(), String> {
        let names: HashSet<_> = selected
            .iter()
            .map(|entry| entry.name.replace('\\', "/"))
            .collect();
        self.verify(source, ctl, |member| {
            if !names.contains(&member.name) {
                return Ok(None);
            }
            let path = stage.join(validate_name(&member.name)?);
            create_staged_output(&path, guards).map(Some)
        })
    }

    pub(super) fn test(&self, source: &File, ctl: &JobControl) -> Result<(), String> {
        self.verify(source, ctl, |_| Ok(None))
    }

    fn verify(
        &self,
        source: &File,
        ctl: &JobControl,
        mut output: impl FnMut(&Member) -> Result<Option<File>, String>,
    ) -> Result<(), String> {
        let mut buffer = [0; 64 * 1024];
        for member in &self.members {
            ctl.check()?;
            let size = member
                .fragments
                .iter()
                .try_fold(0u64, |size, part| size.checked_add(part.size));
            if size != Some(member.size) {
                return Err(format!("classic RAR stored size mismatch: {}", member.name));
            }
            let mut output = output(member)?;
            let mut full_crc = crc32fast::Hasher::new();
            for (index, part) in member.fragments.iter().enumerate() {
                let mut volume = if part.volume == 0 {
                    source
                } else {
                    &self.later[part.volume - 1]
                };
                volume
                    .seek(SeekFrom::Start(part.start))
                    .map_err(|e| e.to_string())?;
                let mut remaining = part.size;
                // Pre-2.0 records may repeat the whole-file CRC in an earlier
                // volume. Any other value must match that fragment's bytes.
                let mut packed_crc = (index + 1 != member.fragments.len()
                    && part.crc != member.crc)
                    .then(crc32fast::Hasher::new);
                while remaining != 0 {
                    ctl.check()?;
                    let size = remaining.min(buffer.len() as u64) as usize;
                    let chunk = &mut buffer[..size];
                    volume.read_exact(chunk).map_err(|e| e.to_string())?;
                    if let Some(crc) = &mut packed_crc {
                        crc.update(chunk);
                    }
                    full_crc.update(chunk);
                    if let Some(file) = &mut output {
                        file.write_all(chunk).map_err(|e| e.to_string())?;
                    }
                    remaining -= size as u64;
                }
                if packed_crc.is_some_and(|crc| part.crc != crc.finalize()) {
                    return Err(format!("classic RAR packed CRC mismatch: {}", member.name));
                }
            }
            if full_crc.finalize() != member.crc {
                return Err(format!("classic RAR file CRC mismatch: {}", member.name));
            }
        }
        ctl.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_volume_headers_share_one_metadata_budget() {
        const FIRST: &[u8] = include_bytes!("../../tests/fixtures/deep/rar15/split-store.rar");
        const LATER: &[u8] = include_bytes!("../../tests/fixtures/deep/rar15/split-store.r00");
        let mut budget = HeaderBudget {
            count: 3,
            bytes: 112,
        };
        let read = |bytes, budget: &HeaderBudget| {
            ArchiveReader::read_reader_with_options(io::Cursor::new(bytes), budget.options())
        };
        let first = read(FIRST, &budget).unwrap();
        let first = first.as_rar15_40().unwrap();
        budget.charge(first.main.head_size.into()).unwrap();
        budget
            .charge(first.files().next().unwrap().block.head_size.into())
            .unwrap();
        assert!(matches!(
            read(LATER, &budget).unwrap_err().root_cause(),
            rars::Error::HeaderCountLimitExceeded { .. }
        ));
        budget.count = 2;
        assert!(read(LATER, &budget).is_ok());
        budget.bytes -= 1;
        assert!(matches!(
            read(LATER, &budget).unwrap_err().root_cause(),
            rars::Error::HeaderBytesLimitExceeded { .. }
        ));
    }
}
