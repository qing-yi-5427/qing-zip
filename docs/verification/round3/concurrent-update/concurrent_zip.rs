use obz_core::engine::{self, EngineOptions};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 4, "two distinct archive/input pairs are required");
    assert_ne!(args[0], args[2]);
    let gate = Arc::new(Barrier::new(3));
    let threads: Vec<_> = args
        .chunks_exact(2)
        .map(|pair| {
            let archive = pair[0].clone();
            let input = pair[1].clone();
            let gate = gate.clone();
            thread::spawn(move || {
                gate.wait();
                let started = Instant::now();
                engine::update(&archive, "add", &[input], &EngineOptions::default()).unwrap();
                started.elapsed().as_secs_f64() * 1000.0
            })
        })
        .collect();
    let started = Instant::now();
    gate.wait();
    let durations: Vec<_> = threads.into_iter().map(|worker| worker.join().unwrap()).collect();
    println!(
        "{{\"two_jobs_ms\":{:.6},\"job_ms\":[{:.6},{:.6}]}}",
        started.elapsed().as_secs_f64() * 1000.0,
        durations[0],
        durations[1],
    );
}
