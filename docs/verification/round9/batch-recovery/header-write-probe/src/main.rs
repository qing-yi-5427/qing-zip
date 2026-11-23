use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::Path,
    time::Instant,
};

struct CountWrites<'a> {
    file: &'a mut File,
    writes: usize,
}

impl Write for CountWrites<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn plan(count: usize) -> Value {
    let entries: Vec<_> = (0..count)
        .map(|index| json!({
            "path": format!(r"G:\QingZipQA\isolated\input\example-{index}.zip"),
            "dest": format!(r"G:\QingZipQA\isolated\output\{index}-example"),
            "status": "pending",
            "hasOutput": false,
        }))
        .collect();
    json!({
        "version": 2,
        "revision": "00000000-0000-4000-8000-000000000002",
        "plan": {
            "destination": r"G:\QingZipQA\isolated\output",
            "overwrite": "error",
            "entries": entries,
        },
    })
}

fn write_record(writer: &mut impl Write, record: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn measure(path: &Path, record: &Value, buffered: bool) -> io::Result<(f64, usize, u64)> {
    let start = Instant::now();
    let mut file = File::create_new(path)?;
    let mut counter = CountWrites { file: &mut file, writes: 0 };
    if buffered {
        write_record(&mut BufWriter::new(&mut counter), record)?;
    } else {
        write_record(&mut counter, record)?;
    }
    counter.file.sync_all()?;
    let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
    Ok((milliseconds, counter.writes, counter.file.metadata()?.len()))
}

fn main() -> io::Result<()> {
    let folder = std::env::args_os().nth(1).expect("provide a new temporary directory");
    let folder = Path::new(&folder);
    for count in [3, 24, 512] {
        let record = plan(count);
        for round in 0..8 {
            let paths = [
                folder.join(format!("{count}-{round}-plain.json")),
                folder.join(format!("{count}-{round}-buf.json")),
            ];
            let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
            let mut trials = [(0.0, 0, 0); 2];
            for index in order {
                trials[index] = measure(&paths[index], &record, index == 1)?;
            }
            assert_eq!(fs::read(&paths[0])?, fs::read(&paths[1])?);
            let [plain, buffered] = trials;
            println!("{}", json!({
                "n": count, "round": round, "warmup": round == 0, "bytes": plain.2,
                "unbuffered_ms": plain.0, "buffered_ms": buffered.0,
                "underlying_file_writes": [plain.1, buffered.1], "same_bytes": true,
            }));
        }
    }
    Ok(())
}
