//! Diagnostic-only cumulative timings. Nested phases overlap; never sum them.
use std::{collections::BTreeMap,sync::{Mutex,OnceLock},time::Instant};
static STATS: OnceLock<Mutex<BTreeMap<&'static str,(u64,u128)>>> = OnceLock::new();
pub struct Span(&'static str,Instant);
impl Span {pub fn new(name:&'static str)->Self{Self(name,Instant::now())}}
impl Drop for Span {fn drop(&mut self){let elapsed=self.1.elapsed().as_nanos();let mut stats=STATS.get_or_init(Default::default).lock().unwrap();let value=stats.entry(self.0).or_default();value.0+=1;value.1+=elapsed;}}
pub struct Report;
impl Drop for Report {fn drop(&mut self){if let Some(stats)=STATS.get(){for (name,(count,ns)) in stats.lock().unwrap().iter(){eprintln!("OUTPUT_PHASE {name} count={count} ns={ns}");}}}}
