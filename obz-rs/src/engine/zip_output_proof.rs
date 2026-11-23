//! Full-byte archive evidence from the successful writes to a private ZIP.
use super::JobControl;
use std::{
    fs::File,
    io::{Read, Write},
    os::windows::fs::FileExt,
    thread::JoinHandle,
};

const BLOCK: usize = 256 * 1024;
const MAX_BLOCKS: usize = 16384; // the worker handles classic ZIP only
pub(super) const FRAME_LIMIT: usize = MAX_BLOCKS * 33;
pub(super) const FRAME_KIND: u8 = 59;
const DOMAIN: &[u8] = b"qing-zip/output-block-proof-v1\0";
type Digest = [u8; 32];

pub(super) enum Prepared<'a> {
    Blocks(&'a Written),
    Streamed { digest: Digest, length: u64 },
}

pub(super) fn capture_stdout(
    mut destination: File,
    control: JobControl,
) -> Result<(File, JoinHandle<Result<Prepared<'static>, String>>), String> {
    let (mut reader, writer) = super::directory_capture::pipe().map_err(|e| e.to_string())?;
    let task = std::thread::Builder::new()
        .name("zip-output-proof".into())
        .spawn(move || {
            let mut buffer = vec![0; BLOCK];
            let mut hasher = blake3::Hasher::new();
            let mut length = 0;
            loop {
                control.check()?;
                let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                destination
                    .write_all(&buffer[..count])
                    .map_err(|e| format!("cannot write ZIP candidate: {e}"))?;
                hasher.update(&buffer[..count]);
                length += count as u64;
            }
            Ok(Prepared::Streamed {
                digest: *hasher.finalize().as_bytes(),
                length,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok((writer, task))
}

#[derive(Default)]
pub(super) struct Written {
    blocks: Vec<Option<Digest>>,
    current: Option<Partial>,
    unsupported: bool,
}

struct Partial {
    index: usize,
    len: usize,
    digest: blake3::Hasher,
}

impl Written {
    pub(super) fn write(&mut self, at: u64, mut bytes: &[u8]) {
        if bytes.is_empty() || self.unsupported {
            return;
        }
        let Some(end) = at.checked_add(bytes.len() as u64) else {
            self.unsupported = true;
            return;
        };
        if end > (BLOCK * MAX_BLOCKS) as u64 {
            self.unsupported = true;
            return;
        }
        let count = (end as usize).div_ceil(BLOCK);
        if self.blocks.len() < count {
            self.blocks.resize(count, None);
        }
        let mut position = at as usize;
        while !bytes.is_empty() {
            let index = position / BLOCK;
            let within = position % BLOCK;
            let take = bytes.len().min(BLOCK - within);
            let (part, rest) = bytes.split_at(take);
            self.blocks[index] = None;
            if within == 0 {
                self.current = Some(Partial {
                    index,
                    len: 0,
                    digest: blake3::Hasher::new(),
                });
            }
            if let Some(current) = self.current.as_mut().filter(|block| block.index == index) {
                if within == current.len {
                    current.digest.update(part);
                    current.len += take;
                    if current.len == BLOCK {
                        self.blocks[index] = Some(*current.digest.finalize().as_bytes());
                        self.current = None;
                    }
                } else {
                    self.current = None;
                }
            }
            bytes = rest;
            position += take;
        }
    }

    pub(super) fn resize(&mut self, length: u64) {
        // Never reuse evidence for a truncated final block or bytes exposed
        // again by a later enlargement. A subsequent full rewrite can prove it.
        if length <= (BLOCK * MAX_BLOCKS) as u64 {
            let keep = length as usize / BLOCK;
            self.blocks.truncate(keep);
            if self
                .current
                .as_ref()
                .is_some_and(|current| current.index >= keep)
            {
                self.current = None;
            }
        }
    }

    pub(super) fn serialize(&self) -> Vec<u8> {
        if self.unsupported {
            return Vec::new();
        }
        let mut body = Vec::with_capacity(self.blocks.len() * 33);
        for block in &self.blocks {
            match block {
                Some(digest) => {
                    body.push(1);
                    body.extend_from_slice(digest);
                }
                None => body.extend_from_slice(&[0; 33]),
            }
        }
        body
    }

    pub(super) fn deserialize(body: &[u8]) -> Result<Self, String> {
        if body.len() > FRAME_LIMIT || body.len() % 33 != 0 {
            return Err("invalid owned ZIP block proof length".into());
        }
        let mut blocks = Vec::with_capacity(body.len() / 33);
        for cell in body.chunks_exact(33) {
            blocks.push(match cell[0] {
                0 if cell[1..].iter().all(|&byte| byte == 0) => None,
                1 => Some(cell[1..].try_into().expect("fixed digest length")),
                _ => return Err("invalid owned ZIP block proof".into()),
            });
        }
        Ok(Self {
            blocks,
            ..Self::default()
        })
    }

    pub(super) fn seal(
        &self,
        file: &File,
        length: u64,
        control: &JobControl,
    ) -> Result<(Digest, u64), String> {
        let mut root = blake3::Hasher::new();
        root.update(DOMAIN);
        root.update(&length.to_le_bytes());
        let mut buffer = vec![0; BLOCK];
        let mut position = 0;
        let mut read_bytes = 0;
        while position < length {
            control.check()?;
            let size = (length - position).min(BLOCK as u64) as usize;
            let block = (size == BLOCK)
                .then(|| {
                    self.blocks
                        .get((position / BLOCK as u64) as usize)
                        .copied()
                        .flatten()
                })
                .flatten()
                .filter(|_| !self.unsupported);
            let digest = match block {
                Some(digest) => digest,
                None => {
                    let mut filled = 0;
                    while filled < size {
                        let used = file
                            .seek_read(&mut buffer[filled..size], position + filled as u64)
                            .map_err(|err| err.to_string())?;
                        if used == 0 {
                            return Err("candidate length changed".into());
                        }
                        filled += used;
                    }
                    read_bytes += size as u64;
                    *blake3::hash(&buffer[..size]).as_bytes()
                }
            };
            root.update(&digest);
            position += size as u64;
        }
        Ok((*root.finalize().as_bytes(), read_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    fn confirmed(file: &File, written: &Written) -> u64 {
        let len = file.metadata().unwrap().len();
        let control = JobControl::new();
        let proof = Written::deserialize(&written.serialize()).unwrap();
        let (value, reread) = proof.seal(file, len, &control).unwrap();
        assert_eq!(
            value,
            Written::default().seal(file, len, &control).unwrap().0
        );
        reread
    }

    #[test]
    fn full_sequential_blocks_need_only_the_tail_read() {
        let mut file = tempfile::tempfile().unwrap();
        let mut written = Written::default();
        let bytes = vec![b'Q'; 3 * BLOCK + 99];
        let mut position = 0;
        for bytes in bytes.chunks(4549) {
            file.write_all(bytes).unwrap();
            written.write(position, bytes);
            position += bytes.len() as u64;
        }
        assert_eq!(confirmed(&file, &written), 99);
    }

    #[test]
    fn rewritten_truncated_and_unwritten_ranges_are_read_again() {
        let mut file = tempfile::tempfile().unwrap();
        let mut written = Written::default();
        let bytes = vec![b'Q'; 4 * BLOCK];
        file.write_all(&bytes).unwrap();
        written.write(0, &bytes);
        file.seek(SeekFrom::Start(7)).unwrap();
        file.write_all(b"updated header").unwrap();
        written.write(7, b"updated header");
        file.set_len(2 * BLOCK as u64 + 7).unwrap();
        written.resize(2 * BLOCK as u64 + 7);
        file.set_len(3 * BLOCK as u64 + 15).unwrap();
        written.resize(3 * BLOCK as u64 + 15);
        assert_eq!(confirmed(&file, &written), 2 * BLOCK as u64 + 15);

        // An exact subsequent full rewrite can prove the previously dirty block.
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&bytes[..BLOCK]).unwrap();
        written.write(0, &bytes[..BLOCK]);
        assert_eq!(confirmed(&file, &written), BLOCK as u64 + 15);
    }

    #[test]
    fn console_capture_commits_exactly_the_binary_payload() {
        let mut file = tempfile::tempfile().unwrap();
        let (mut writer, capture) =
            capture_stdout(file.try_clone().unwrap(), JobControl::new()).unwrap();
        let bytes: Vec<_> = (0..(2 * BLOCK + 5)).map(|index| index as u8).collect();
        writer.write_all(&bytes).unwrap();
        drop(writer);
        let Prepared::Streamed { digest, length } = capture.join().unwrap().unwrap() else {
            panic!("console output must produce a sequential byte proof");
        };
        assert_eq!(length, bytes.len() as u64);
        assert_eq!(digest, *blake3::hash(&bytes).as_bytes());
        file.rewind().unwrap();
        let mut actual = Vec::new();
        file.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, bytes);
    }
}
