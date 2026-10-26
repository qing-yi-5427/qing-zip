//! Observes existing JobControl phase transitions; does not change extraction.
use obz_core::engine::{self, EngineOptions, JobControl};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};
fn main(){
 let args:Vec<String>=std::env::args().collect();assert_eq!(args.len(),3);
 let ctl=JobControl::new();let done=Arc::new(AtomicBool::new(false));let start=Instant::now();
 let observer_ctl=ctl.clone();let observer_done=done.clone();
 let observer=thread::spawn(move||{let mut transitions=Vec::new();let mut previous=String::new();while !observer_done.load(Ordering::Relaxed){let p=observer_ctl.snapshot();if p.phase!=previous{previous=p.phase.clone();transitions.push(serde_json::json!({"seconds":start.elapsed().as_secs_f64(),"phase":p.phase,"message":p.message}));}thread::sleep(Duration::from_millis(5));}transitions});
 let opts=EngineOptions{control:Some(ctl),..Default::default()};let result=engine::extract(&args[1],&args[2],&opts);let elapsed=start.elapsed().as_secs_f64();done.store(true,Ordering::Relaxed);let transitions=observer.join().unwrap();println!("{}",serde_json::json!({"total_seconds":elapsed,"result":result,"transitions":transitions,"limitations":"5ms phase observation; decoding phase includes decoder and stage verification, not isolated per-function profiler."}));
}
