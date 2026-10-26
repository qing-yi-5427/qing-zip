//! Temporary controlled performance probe; does not alter extraction behavior.
use chrono::{Local, NaiveDateTime, TimeZone};
use std::{hint::black_box, time::Instant};
fn measure(mut f: impl FnMut(), n: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..n { f(); }
    start.elapsed().as_secs_f64()
}
fn main() {
    let path = std::env::args().nth(1).expect("existing synthetic file path");
    let raw = "2026-10-25 16:20:30.1234567";
    let date = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f").unwrap();
    let n = 6000;
    let parse = measure(|| { black_box(NaiveDateTime::parse_from_str(black_box(raw), "%Y-%m-%d %H:%M:%S%.f").unwrap()); }, n);
    let local = measure(|| { black_box(Local.from_local_datetime(black_box(&date)).earliest().unwrap()); }, n);
    let offset = *Local.from_local_datetime(&date).earliest().unwrap().offset();
    let fixed = measure(|| { black_box(offset.from_local_datetime(black_box(&date)).earliest().unwrap()); }, n);
    let allocations = measure(|| { black_box(vec![0u8; 1024*1024]); }, n);
    let metadata = measure(|| { black_box(std::fs::symlink_metadata(black_box(&path)).unwrap()); }, n);
    #[cfg(windows)]
    let attributes = {
        use std::os::windows::ffi::OsStrExt;
        #[link(name="Kernel32")] extern "system" { fn GetFileAttributesW(path:*const u16)->u32; }
        let wide:Vec<u16>=std::ffi::OsStr::new(&path).encode_wide().chain(Some(0)).collect();
        measure(|| { let v=unsafe{GetFileAttributesW(black_box(wide.as_ptr()))}; assert_ne!(v,u32::MAX);black_box(v); },n)
    };
    println!("{}",serde_json::json!({"iterations":n,"date":raw,"date_parse_seconds":parse,"chrono_local_seconds":local,"fixed_offset_seconds":fixed,"one_mib_buffer_allocation_seconds":allocations,"std_symlink_metadata_seconds":metadata,"win32_attributes_seconds":attributes,"fixed_offset_warning":"Only a microbenchmark comparison. A single fixed offset is NOT a correct general replacement across DST/history."}));
}
