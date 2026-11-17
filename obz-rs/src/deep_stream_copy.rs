//! Fixed-memory sink used only by deep extraction's raw-stream decoder.
use std::io::{Read, Write};

#[derive(Debug)]
pub struct Attempt {
    pub bytes: u64,
    pub error: Option<String>,
}

pub fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    limit: u64,
    check: &mut impl FnMut(u64) -> Result<(), String>,
) -> Attempt {
    let mut bytes = 0u64;
    let result = (|| {
        let mut buffer = [0u8; 65536];
        loop {
            check(0)?;
            let request = limit.saturating_sub(bytes).max(1).min(buffer.len() as u64) as usize;
            let count = reader
                .read(&mut buffer[..request])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                writer.flush().map_err(|e| e.to_string())?;
                return Ok(());
            }
            if count as u64 > limit.saturating_sub(bytes) {
                return Err("已达到累计解码量限制；未写入超限数据".into());
            }
            check(count as u64)?;
            // Account each successful write even if a later write/flush fails.
            let mut position = 0;
            while position < count {
                let written = writer
                    .write(&buffer[position..count])
                    .map_err(|e| e.to_string())?;
                if written == 0 {
                    return Err("无法写入解码结果".into());
                }
                position += written;
                bytes += written as u64;
            }
        }
    })();
    Attempt {
        bytes,
        error: result.err(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_boundary_and_excess_never_publish_an_extra_byte() {
        for size in [0, 10, 11, 1_000_000] {
            let mut input = std::io::Cursor::new(vec![7; size]);
            let mut out = Vec::new();
            let result = copy_bounded(&mut input, &mut out, 10, &mut |_| Ok(()));
            assert_eq!(out.len(), size.min(10));
            assert_eq!(result.bytes, out.len() as u64);
            assert_eq!(result.error.is_some(), size > 10);
        }
    }
    #[test]
    fn cancellation_and_free_reserve_are_checked_before_writes() {
        for message in ["cancelled", "low disk space"] {
            let mut input = std::io::Cursor::new(vec![7; 100]);
            let mut out = Vec::new();
            let result = copy_bounded(&mut input, &mut out, 100, &mut |next| {
                if next > 0 {
                    Err(message.into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.bytes, 0);
            assert!(out.is_empty());
            assert_eq!(result.error.as_deref(), Some(message));
        }
    }
    #[test]
    fn failed_attempt_keeps_partial_write_accounting() {
        struct Partial {
            bytes: usize,
        }
        impl Write for Partial {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.bytes > 0 {
                    return Err(std::io::Error::other("mock failure"));
                }
                self.bytes = bytes.len().min(3);
                Ok(self.bytes)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Partial { bytes: 0 };
        let result = copy_bounded(
            &mut std::io::Cursor::new(vec![7; 10]),
            &mut writer,
            10,
            &mut |_| Ok(()),
        );
        assert_eq!(result.bytes, 3);
        assert!(result.error.is_some());
    }
}
