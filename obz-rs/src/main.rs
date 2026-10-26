//! obz - qing-zip native CLI.
//!
//! Native setup package inspection/extraction and generic zip/7z archive
//! operations. Read-only against inputs; nothing is ever executed.

use obz_core::{setup, zipfmt, operations};

use std::fs;
use std::process::ExitCode;

const USAGE: &str = "\
obz - qing-zip native toolkit

USAGE:
  obz setup info    <setup.exe>
  obz setup list    <setup.exe> [--ui]
  obz setup extract <setup.exe> <dest> [--ui | --all]
  obz setup plan    <setup.exe>
  obz list|test      <archive> [options]
  obz extract       <archive> <dest> [--include entry] [--smart] [options]
  obz create        <out> <path...> [--format zip|7z|zipx|tar|tgz|tbz2|txz|gzip|xz|bzip2|wim|sfx|iso|lzh]
  obz add|delete    <archive> <paths...>
  obz rename        <archive> <old> <new> ...
  obz preview       <archive> <entry>       (image data URL as JSON)
  obz repair        <broken.zip> <new.zip>  (keeps original)
  obz recover       <archive> --candidates <file> --max-attempts 1000
  obz checksum      <file>                  (CRC32/MD5/SHA1/SHA256)
  obz vault-list | vault-put <id> --label <label> --password-stdin | vault-delete <id>
  obz shell-enable|shell-disable <qing-zip.exe>
  obz capabilities
  obz json [request.json]                  (otherwise read JSON from stdin)

Options: --password-stdin, --password-id ID, --level 0..9, --threads N,
         --volume BYTES, --codepage N, --overwrite error|skip|rename|overwrite,
         --store, --include ENTRY (repeatable); -- ends option parsing.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print!("{USAGE}");
        return Ok(());
    }
    match args[0].as_str() {
        "setup" => {
            let rest = &args[1..];
            match rest.first().map(|s| s.as_str()) {
                Some("info") => cmd_setup_info(&rest[1..]),
                Some("list") => cmd_setup_list(&rest[1..]),
                Some("extract") => cmd_setup_extract(&rest[1..]),
                Some("plan") => cmd_setup_plan(&rest[1..]),
                _ => Err(format!("unknown setup subcommand\n{USAGE}")),
            }
        }
        "list" | "extract" | "create" | "test" | "add" | "delete" | "rename" | "preview" | "repair" | "checksum" | "recover" | "capabilities" | "vault-list" | "vault-put" | "vault-delete" | "shell-enable" | "shell-disable" => cmd_operation(args),
        "json" => cmd_json(&args[1..]),
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn cmd_setup_info(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("setup info needs <setup.exe>")?;
    let data = read_file(path)?;
    let pkg = setup::open(&data)?;

    println!("file          {path}");
    println!("size          {} bytes", data.len());
    println!(
        "stub          PE32{} {}  sections={}  overlay@{}",
        if pkg.pe.pe32plus { "+" } else { "" },
        pkg.pe.machine_name,
        pkg.pe.sections.len(),
        pkg.pe.overlay_offset
    );
    for s in &pkg.pe.sections {
        println!(
            "  section     {:<8} raw={:>10}+{:<10} virt={:#x}+{:#x}",
            s.name, s.raw_offset, s.raw_size, s.vaddr, s.vsize
        );
    }
    let unpacked: u64 = pkg
        .archive
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.size)
        .sum();
    println!(
        "7z payload    offset={}  entries={}  folders={}",
        pkg.sevenz_offset,
        pkg.archive.entries.len(),
        pkg.archive.folders.len()
    );
    println!(
        "              unpacked={} bytes  encoded_header={}",
        unpacked, pkg.archive.encoded_header
    );
    println!(
        "ui zip        offset={}  entries={}",
        pkg.zip_offset,
        pkg.ui_entries.len()
    );
    if let Some(c) = &pkg.cert {
        println!(
            "certificate   offset={}  length={}  revision={:#x}  type={:#x}",
            c.offset, c.length, c.revision, c.cert_type
        );
    }
    Ok(())
}

fn cmd_setup_list(args: &[String]) -> Result<(), String> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or("setup list needs <setup.exe>")?;
    let ui = args.iter().any(|a| a == "--ui");
    let data = read_file(path)?;
    let pkg = setup::open(&data)?;
    if ui {
        for e in &pkg.ui_entries {
            let m = if e.method == 8 { "deflate" } else { "store" };
            println!("{:>12}  {:<8}  {}", e.size, m, e.name);
        }
    } else {
        for e in &pkg.archive.entries {
            let kind = if e.is_dir { "dir " } else { "file" };
            let crc = match e.crc {
                Some(c) => format!("{c:08x}"),
                None => "-".into(),
            };
            println!("{:>12}  {}  crc={}  {}", e.size, kind, crc, e.name);
        }
    }
    Ok(())
}

fn cmd_setup_extract(args: &[String]) -> Result<(), String> {
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (path, dest) = match pos.as_slice() {
        [p, d] => (*p, *d),
        _ => return Err("setup extract needs <setup.exe> <dest>".into()),
    };
    let ui_only = args.iter().any(|a| a == "--ui");
    let all = args.iter().any(|a| a == "--all");
    let data = read_file(path)?;
    let pkg = setup::open(&data)?;
    let mut n = 0;
    if ui_only || all {
        n += setup::extract_ui(&pkg, &format!("{dest}/ui"))?;
    }
    if !ui_only || all {
        n += setup::extract_payload(&pkg, &format!("{dest}/payload"))?;
    }
    println!("extracted {n} files under {dest}");
    Ok(())
}

fn cmd_setup_plan(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("setup plan needs <setup.exe>")?;
    let data = read_file(path)?;
    let pkg = setup::open(&data)?;
    for e in &pkg.ui_entries {
        if !(e.name.ends_with(".xs") || e.name.ends_with(".xml") || e.name.ends_with(".config")) {
            continue;
        }
        let blob = zipfmt::extract_entry(pkg.data, pkg.zip_offset, e)?;
        let text = String::from_utf8_lossy(&blob);
        println!("== {} ==", e.name);
        if e.name.ends_with(".xml") {
            let mut includes = 0;
            for line in text.lines() {
                if let Some(i) = line.find("<script src=\"") {
                    let rest = &line[i + 12..];
                    if let Some(j) = rest.find('"') {
                        println!("  include {}", &rest[..j]);
                        includes += 1;
                    }
                }
            }
            let tags = text.matches('<').count();
            println!("  xml tags ~{tags}  includes={includes}");
        } else if e.name.ends_with(".xs") {
            for line in text.lines() {
                let t = line.trim();
                if t.starts_with("void ") && t.contains('(') {
                    let name = t.trim_start_matches("void ");
                    let name = name.split('(').next().unwrap_or("");
                    println!("  fn {name}");
                } else if t.starts_with("string ") || t.starts_with("int ") {
                    if let Some(eq) = t.find('=') {
                        println!("  const {}", t[..eq].trim());
                    }
                }
            }
        } else {
            for line in text.lines().filter(|l| !l.trim().is_empty()).take(40) {
                println!("  {}", line.trim());
            }
        }
    }
    Ok(())
}


fn cmd_json(args: &[String]) -> Result<(), String> {
    use std::io::Read;
    let text = if let Some(path) = args.first() { fs::read_to_string(path).map_err(|e|e.to_string())? }
        else { let mut s=String::new(); std::io::stdin().take(16*1024*1024).read_to_string(&mut s).map_err(|e|e.to_string())?; s };
    let request = serde_json::from_str(&text).map_err(|e|format!("invalid request: {e}"))?;
    let result = operations::execute(request)?;
    println!("{}", serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
    Ok(())
}

fn cmd_operation(args: &[String]) -> Result<(), String> {
    let mut r = operations::Request { operation: args[0].clone(), ..Default::default() };
    let mut pos = Vec::new(); let mut i=1; let mut literal=false;
    while i<args.len() {
        let a=&args[i];
        if !literal && a=="--" { literal=true; i+=1; continue; }
        if !literal && a.starts_with("--") {
            match a.as_str() {
                "--store" => r.options.level=Some(0),
                "--smart" => r.smart=true,
                "--password-stdin" => {let mut p=String::new(); std::io::stdin().read_line(&mut p).map_err(|e|e.to_string())?; r.options.password=Some(p.trim_end_matches(['\r','\n']).to_string());},
                "--format"|"--password-id"|"--codepage"|"--threads"|"--level"|"--volume"|"--include"|"--overwrite"|"--name"|"--label"|"--candidates"|"--max-attempts"|"--charset"|"--min-length"|"--max-length" => {
                    i+=1; let v=args.get(i).ok_or_else(||format!("{a} requires a value"))?;
                    match a.as_str() {
                        "--charset"=>r.charset=Some(v.clone()),
                        "--min-length"=>r.min_length=Some(v.parse().map_err(|_|"invalid minimum length")?),
                        "--max-length"=>r.max_length=Some(v.parse().map_err(|_|"invalid maximum length")?),
                        "--format"=>r.format=v.clone(), "--password-id"=>r.password_id=Some(v.clone()),
                        "--codepage"=>r.options.codepage=Some(v.parse().map_err(|_|"invalid codepage")?),
                        "--threads"=>r.options.threads=Some(v.parse().map_err(|_|"invalid threads")?),
                        "--level"=>r.options.level=Some(v.parse().map_err(|_|"invalid level")?),
                        "--volume"=>r.options.volume_bytes=Some(v.parse().map_err(|_|"volume must be bytes")?),
                        "--include"=>r.options.selected.push(v.clone()), "--overwrite"=>r.options.overwrite=v.clone(),
                        "--name"=>r.name=v.clone(), "--label"=>r.label=v.clone(),
                        "--max-attempts"=>r.max_attempts=Some(v.parse().map_err(|_|"invalid max attempts")?),
                        "--candidates"=>r.candidates=fs::read_to_string(v).map_err(|e|e.to_string())?.lines().map(str::to_owned).collect(),
                        _=>unreachable!(),
                    }
                },
                _=>return Err(format!("unknown option {a}")),
            }
        } else { pos.push(a.clone()); }
        i+=1;
    }
    match r.operation.as_str() {
        "capabilities"|"vault-list"=>{},
        "vault-put"|"vault-delete"=> {r.name=pos.first().ok_or("vault item ID required")?.clone();},
        _=>r.path=pos.first().ok_or("archive path required")?.clone(),
    }
    match r.operation.as_str() {
        "extract"|"repair"=>r.dest=pos.get(1).ok_or("destination required")?.clone(),
        "create"|"add"|"delete"|"rename"=>r.inputs=pos.into_iter().skip(1).collect(),
        "preview"=>{r.name=pos.get(1).ok_or("entry name required")?.clone();},
        _=>{},
    }
    let result=operations::execute(r)?;
    println!("{}",serde_json::to_string_pretty(&result).map_err(|e|e.to_string())?);
    Ok(())
}
