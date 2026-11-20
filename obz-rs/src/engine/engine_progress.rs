use super::{JobControl, ProgressWork};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

const WINDOW: usize = 4096;

pub(super) struct LiveProgress {
    command: &'static str,
    frames: Option<File>,
    output: Option<(File, u64)>,
    read_end: u64,
}

impl LiveProgress {
    pub(super) fn open(
        command: Option<&str>,
        errors: &tempfile::NamedTempFile,
        output: Option<(&File, u64)>,
    ) -> Option<Self> {
        let command = match command {
            Some("x" | "e") => "x",
            Some("t") => "t",
            Some("a" | "u") => "a",
            Some("d" | "rn") => "d",
            _ => return None,
        };
        // Obtain a separate position on the same file by handle. Seeking a
        // duplicated writer handle would move the child's stderr position too.
        let frames = errors.reopen().ok();
        let output = output
            .filter(|(_, size)| *size > 0)
            .and_then(|(file, size)| Some((file.try_clone().ok()?, size)));
        Some(Self {
            command,
            frames,
            output,
            read_end: 0,
        })
    }

    fn engine_percent(&mut self) -> Option<u8> {
        let frames = self.frames.as_mut()?;
        let end = frames.metadata().ok()?.len();
        if end <= self.read_end {
            return None;
        }
        let start = end.saturating_sub(WINDOW as u64);
        let mut tail = [0; WINDOW];
        let bytes = &mut tail[..(end - start) as usize];
        frames.seek(SeekFrom::Start(start)).ok()?;
        frames.read_exact(bytes).ok()?;
        self.read_end = end;
        latest_percent(bytes, start == 0)
    }

    pub(super) fn sample(&mut self, ctl: &JobControl) {
        let percent = self.engine_percent();
        // A single member is streamed to the original open stage handle. Its
        // length is the actual byte count written so far, independent of how
        // often the decoder reports a percent or whether the RAR is solid.
        let output = self.output.as_ref().and_then(|(file, total)| {
            let done = file.metadata().ok()?.len();
            (done <= *total).then_some((done, *total))
        });
        if percent.is_none() && output.is_none() {
            return;
        }
        let mut state = ctl.progress.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(
            (self.command, state.phase.as_str()),
            ("x", "decoding" | "decompressing")
                | ("t", "testing")
                | ("a", "compressing" | "updating")
                | ("d", "updating")
        ) {
            return;
        }
        if let Some((done, total)) = output {
            let measured = ((u128::from(done) * 100) / u128::from(total)) as u8;
            let percent = if done == 0 {
                percent.unwrap_or(0)
            } else {
                measured
            };
            state.percent = Some(percent.max(state.percent.unwrap_or(0)));
            // The decoder must still exit successfully before the member is
            // proven complete. Verification and publication report separately.
            state.work = Some(ProgressWork {
                items_done: 0,
                items_total: 1,
                bytes_done: Some(done),
                bytes_total: Some(total),
            });
        } else if let Some(percent) = percent {
            if state.work.is_none() && state.percent.is_none_or(|previous| percent >= previous) {
                state.percent = Some(percent);
            }
        }
    }
}

// The redirected 7-Zip/NanaZip console displays a fixed three-column
// percentage and a percent sign, then optional counters and a member name.
// It terminates display and erase frames with carriage returns. Parse only
// complete frames, leaving the member-name bytes (which may be partial UTF-8)
// and regular stderr diagnostics out of the calculation.
fn latest_percent(bytes: &[u8], starts_at_origin: bool) -> Option<u8> {
    let bytes = if starts_at_origin {
        bytes
    } else {
        let first = bytes.iter().position(|&byte| byte == b'\r')?;
        &bytes[first + 1..]
    };
    for frame in bytes.rsplit(|&byte| byte == b'\r').skip(1) {
        if frame.get(3) != Some(&b'%') || frame.get(4).is_some_and(|&byte| byte != b' ') {
            continue;
        }
        let digits = frame[..3].strip_prefix(b" ").unwrap_or(&frame[..3]);
        let digits = digits.strip_prefix(b" ").unwrap_or(digits);
        if digits.iter().all(u8::is_ascii_digit) {
            let percent = digits
                .iter()
                .fold(0u16, |value, digit| value * 10 + u16::from(digit - b'0'));
            if percent <= 100 {
                return Some(percent as u8);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_real_carriage_return_progress_frames() {
        assert_eq!(latest_percent(b"  0M Scan\r         \r", true), None);
        assert_eq!(
            latest_percent(b"  0%\r    \r 62% 39 T payload99%\xff.bin\r", true),
            Some(62)
        );
        assert_eq!(latest_percent(b"  0%\r    \r 99%", true), Some(0));
        assert_eq!(
            latest_percent(b" 76%\r    \rERROR: 100% failed\r101%\r", false),
            None
        );
        assert_eq!(latest_percent(b" 76%\r    \r100%\r", false), Some(100));
    }

    #[test]
    fn samples_growing_stdout_and_keeps_progress_in_its_own_phase() {
        use std::io::Write;
        let mut errors = tempfile::NamedTempFile::new().unwrap();
        let mut output = tempfile::tempfile().unwrap();
        let ctl = JobControl::new();
        let mut live = LiveProgress::open(Some("x"), &errors, Some((&output, 8))).unwrap();

        ctl.set("listing", None, "");
        errors.write_all(b" 25%\r").unwrap();
        live.sample(&ctl);
        assert_eq!(ctl.snapshot().percent, None);

        ctl.set("decoding", None, "");
        errors.write_all(b" 25%\r").unwrap();
        live.sample(&ctl);
        assert_eq!(ctl.snapshot().percent, Some(25));

        output.write_all(b"abcd").unwrap();
        errors.write_all(b" 35%\r").unwrap();
        live.sample(&ctl);
        let state = ctl.snapshot();
        assert_eq!(state.percent, Some(50));
        let work = state.work.unwrap();
        assert_eq!((work.bytes_done, work.bytes_total), (Some(4), Some(8)));
        assert_eq!(work.items_done, 0);

        ctl.set("publishing-files", Some(75), "");
        errors.write_all(b" 90%\r").unwrap();
        live.sample(&ctl);
        assert_eq!(ctl.snapshot().percent, Some(75));
    }
}
