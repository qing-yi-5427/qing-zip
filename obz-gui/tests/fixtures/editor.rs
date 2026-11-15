//! Harmless controllable editor for lifecycle/argument tests; never used by the product.
#![windows_subsystem = "windows"]
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let child = args.first().is_some_and(|a| a == "--child");
    let Some(path) = args.last().map(PathBuf::from) else {
        return;
    };
    if args.len() != if child { 2 } else { 1 } {
        return;
    }
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("delayed-handoff"))
        && !child
    {
        thread::sleep(Duration::from_millis(2200));
        let _ = fs::write(
            PathBuf::from(format!("{}.forwarded", path.display())),
            "forwarded",
        );
        return;
    }
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("handoff"))
        && !child
    {
        let _ = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--child")
            .arg(&path)
            .spawn();
        return;
    }
    let marker = |suffix: &str| PathBuf::from(format!("{}{suffix}", path.display()));
    let _ = fs::write(marker(".ready"), std::process::id().to_string());
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        if let Ok(command) = fs::read_to_string(marker(".command")) {
            let _ = fs::remove_file(marker(".command"));
            if command == "exit" {
                break;
            }
            if let Some(content) = command.strip_prefix("write:") {
                let _ = fs::write(&path, content);
                let _ = fs::write(marker(".saved"), "saved");
            }
        }
        thread::sleep(Duration::from_millis(30));
    }
}
