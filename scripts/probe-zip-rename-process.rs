//! Link to a specific product's obz_core to measure ordinary ZIP edits in one process.
use std::io::{self, BufRead, Write};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let path = line?;
        let names = ["ordinary-add.txt".into(), "renamed.bin".into()];
        let options = obz_core::engine::EngineOptions::default();
        let started = Instant::now();
        obz_core::engine::update(&path, "rename", &names, &options)?;
        writeln!(stdout, "{:.3}", started.elapsed().as_secs_f64() * 1000.0)?;
        stdout.flush()?;
    }
    Ok(())
}
