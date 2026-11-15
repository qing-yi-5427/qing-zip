//! The bundled engine's LZMA benchmark, using its own generated data.
use super::{control, run, EngineOptions};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub threads: u32,
    pub dictionary_log: u32,
    pub iterations: u32,
}
impl Default for Options {
    fn default() -> Self { Self { threads: 2, dictionary_log: 24, iterations: 1 } }
}
#[derive(Debug, Serialize, PartialEq)]
pub struct Score {
    pub speed_kib_s: u64,
    pub cpu_percent: u64,
    pub rating_per_usage_mips: u64,
    pub rating_mips: u64,
}
#[derive(Serialize)]
pub struct Report {
    pub parameters: Options,
    pub elapsed_seconds: f64,
    pub compressing: Score,
    pub decompressing: Score,
    pub engine_report: String,
}
fn args(options: &Options) -> Result<Vec<String>, String> {
    if !(1..=256).contains(&options.threads)
        || !(18..=28).contains(&options.dictionary_log)
        || !(1..=20).contains(&options.iterations) {
        return Err("基准测试参数无效：线程 1–256，字典指数 18–28，轮数 1–20".into());
    }
    Ok(vec!["b".into(), options.iterations.to_string(),
        format!("-mmt{}", options.threads), format!("-mdf={}", options.dictionary_log),
        "-bt".into(), "-sccUTF-8".into()])
}
fn scores(report: &str) -> Result<(Score, Score), String> {
    let line = report.lines().find_map(|s| s.trim().strip_prefix("Avr:"))
        .ok_or("引擎未返回完整的基准测试结果")?;
    let (compressing, decompressing) = line.split_once('|').ok_or("基准测试结果格式无效")?;
    fn score(s: &str) -> Result<Score, String> {
        let values = s.split_whitespace().map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>().map_err(|_| "基准测试数值无效")?;
        let [speed_kib_s, cpu_percent, rating_per_usage_mips, rating_mips] = values[..] else {
            return Err("基准测试结果不完整".into());
        };
        Ok(Score { speed_kib_s, cpu_percent, rating_per_usage_mips, rating_mips })
    }
    Ok((score(compressing)?, score(decompressing)?))
}
pub fn execute(parameters: Options, options: &EngineOptions) -> Result<Report, String> {
    let args = args(&parameters)?;
    // Archive passwords, selected names and other unrelated options are never forwarded.
    let options = EngineOptions { control: options.control.clone(), ..Default::default() };
    let ctl = control(&options);
    ctl.check()?;
    ctl.progress.lock().unwrap_or_else(|e| e.into_inner()).phase = "benchmarking".into();
    let start = Instant::now();
    let output = run(&args, None, &options, None)?;
    let elapsed_seconds = start.elapsed().as_secs_f64();
    ctl.check()?;
    let engine_report = String::from_utf8(output).map_err(|_| "基准测试输出编码无效")?;
    let (compressing, decompressing) = scores(&engine_report)?;
    Ok(Report { parameters, elapsed_seconds, compressing, decompressing, engine_report })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_parameters_before_engine_launch() {
        for (threads, dictionary_log, iterations) in [(0,24,1),(257,24,1),(1,17,1),(1,29,1),(1,24,0),(1,24,21)] {
            assert!(args(&Options { threads, dictionary_log, iterations }).is_err());
        }
        assert_eq!(&args(&Options::default()).unwrap()[..4], &["b","1","-mmt2","-mdf=24"]);
    }
    #[test]
    fn requires_complete_numeric_engine_results() {
        let (c,d)=scores("18: 1 2 3 4 | 5 6 7 8\nAvr: 15006 101 13297 13369 | 137071 100 11091 11085\nTot: 100 12194 12227").unwrap();
        assert_eq!(c.speed_kib_s,15006);
        assert_eq!(d.rating_mips,11085);
        for bad in ["", "18: 1 2 3 4 | 5 6 7 8", "Avr: 1 2 3 | 5 6 7 8", "Avr: 1 2 3 4 | 5 6 ? 8"] {
            assert!(scores(bad).is_err());
        }
    }
}
