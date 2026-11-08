//! Stable JSON command contract shared by CLI and the GUI being designed.
use crate::{engine::{self, EngineOptions}, services};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Request {
    pub operation: String,
    pub path: String,
    pub dest: String,
    pub format: String,
    pub inputs: Vec<String>,
    pub options: EngineOptions,
    pub name: String,
    pub label: String,
    pub password_id: Option<String>,
    pub candidates: Vec<String>,
    pub charset: Option<String>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub max_attempts: Option<usize>,
    pub max_bytes: Option<u64>,
    pub smart: bool,
    pub hashes: Vec<String>,
}

pub fn vault_path() -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?;
    Ok(PathBuf::from(root).join("OpenBandiZip").join("passwords.dpapi"))
}

pub fn list_archive(path:&str,opts:&EngineOptions)->Result<Vec<engine::Entry>,String>{
    let result=list_archive_once(path,opts);
    #[cfg(windows)]
    if result.is_err() && engine::recover_interrupted_edit(std::path::Path::new(path))? {
        return list_archive_once(path,opts);
    }
    result
}
fn list_archive_once(path:&str,opts:&EngineOptions)->Result<Vec<engine::Entry>,String>{
    let policy=crate::machine_policy::scope();
    if policy.restricted() { return engine::list(path, opts); }
    if crate::rare::supports(path){return crate::rare::list(path,opts);}
    if crate::envelopes::is_supported(path){return crate::envelopes::list(path,opts);}
    if crate::stream_formats::is_supported(path){return crate::stream_formats::list(path,opts);}
    if let Some(data)=crate::installer::recognize(path)? {return crate::installer::list(&data);}
    if crate::korean::is_supported(path){crate::korean::list(path,opts)}else if crate::legacy::is_supported(path){crate::legacy::list(path,opts)}else{engine::list(path,opts)}
}
pub fn test_archive(path:&str,opts:&EngineOptions)->Result<(),String>{
    let policy = crate::machine_policy::scope();
    if policy.restricted() { return engine::test(path, opts); }
    if crate::rare::supports(path){return crate::rare::test(path,opts);}
    if crate::envelopes::is_supported(path){return crate::envelopes::test(path,opts);}
    if crate::stream_formats::is_supported(path){return crate::stream_formats::test(path,opts);}
    if let Some(data)=crate::installer::recognize(path)? {return crate::installer::test(&data,opts);}
    if crate::korean::is_supported(path){crate::korean::test(path,opts)}else if crate::legacy::is_supported(path){crate::legacy::test(path,opts)}else{engine::test(path,opts)}
}
pub fn extract_archive(path:&str,dest:&str,opts:&EngineOptions)->Result<usize,String>{
    let policy = crate::machine_policy::scope();
    if policy.restricted() { return engine::extract(path, dest, opts); }
    let mut inherited = opts.clone();
    if inherited.zone_identifier.is_none() { inherited.zone_identifier = Some(crate::motw::archive_context(std::path::Path::new(path))?); }
    let opts = &inherited;
    if crate::rare::supports(path){return crate::rare::extract(path,dest,opts);}
    if crate::envelopes::is_supported(path){return crate::envelopes::extract(path,dest,opts);}
    if crate::stream_formats::is_supported(path){return crate::stream_formats::extract(path,dest,opts);}
    if let Some(data)=crate::installer::recognize(path)? {return crate::installer::extract(&data,dest,opts);}
    if crate::korean::is_supported(path){crate::korean::extract(path,dest,opts)}else if crate::legacy::is_supported(path){crate::legacy::extract(path,dest,opts)}else{engine::extract(path,dest,opts)}
}
pub fn preview_archive(path:&str,name:&str,opts:&EngineOptions)->Result<Vec<u8>,String>{
    let policy = crate::machine_policy::scope();
    if policy.restricted() { return engine::preview(path, name, opts); }
    if crate::rare::supports(path){return crate::rare::preview(path,name,opts);}
    if crate::envelopes::is_supported(path){return crate::envelopes::preview(path,name,opts);}
    if crate::stream_formats::is_supported(path){return crate::stream_formats::preview(path,name,opts);}
    if let Some(data)=crate::installer::recognize(path)? {return crate::installer::preview(&data,name,opts);}
    if crate::korean::is_supported(path){crate::korean::preview(path,name,opts)}else if crate::legacy::is_supported(path){crate::legacy::preview(path,name,opts)}else{engine::preview(path,name,opts)}
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OperationResponse {
    Listing {
        #[serde(serialize_with = "serialize_listing_entries")]
        entries: Vec<engine::Entry>,
    },
    Json(Value),
}

fn serialize_listing_entries<S: serde::Serializer>(entries: &[engine::Entry], serializer: S) -> Result<S::Ok, S::Error> {
    // The public CLI and desktop wire format formerly serialized Value maps
    // in lexical key order. Keep that order while borrowing each entry.
    #[derive(Serialize)]
    struct Row<'a> {
        attributes: &'a str, comment: &'a str, crc: &'a str, encrypted: bool,
        is_dir: bool, method: &'a str, modified: &'a str, name: &'a str,
        packed_size: u64, size: u64, size_known: bool,
    }
    serializer.collect_seq(entries.iter().map(|entry| {
        let engine::Entry { attributes, comment, crc, encrypted, is_dir, method, modified, name, packed_size, size, size_known } = entry;
        Row { attributes, comment, crc, encrypted: *encrypted, is_dir: *is_dir, method,
              modified, name, packed_size: *packed_size, size: *size, size_known: *size_known }
    }))
}

/// Compatibility entry point for callers that inspect or mutate the JSON tree.
pub fn execute(r: Request) -> Result<Value, String> {
    match execute_request(r)? {
        OperationResponse::Listing { entries } => Ok(json!({ "entries": entries })),
        OperationResponse::Json(value) => Ok(value),
    }
}

/// No shell commands or arbitrary application execution are accepted here.
pub fn execute_request(mut r: Request) -> Result<OperationResponse, String> {
    let archive_operation = matches!(r.operation.as_str(), "capabilities" | "list" | "test" | "extract" | "create" | "add" | "delete" | "rename" | "preview" | "repair" | "comment-get" | "comment-set" | "recover");
    let policy = archive_operation.then(crate::machine_policy::scope);
    let restricted = policy.as_ref().is_some_and(|p| p.restricted());
    if let Some(id) = &r.password_id {
        if r.options.password.is_some() { return Err("choose password or password_id, not both".into()); }
        r.options.password = Some(services::Vault::open(vault_path()?)?.get(id)?);
    }
    if r.operation == "list" {
        return Ok(OperationResponse::Listing { entries: list_archive(&r.path, &r.options)? });
    }
    execute_json(r, restricted).map(OperationResponse::Json)
}

fn execute_json(r: Request, restricted: bool) -> Result<Value, String> {
    match r.operation.as_str() {
        "capabilities" => {
            if restricted {
                let mut caps = engine::capabilities();
                caps.read_formats.clear(); caps.write_formats.clear();
                caps.limitations = vec!["Archive handlers and codecs are controlled by the NanaZip machine policy. The bundled official engine checks the actual format and nested methods.".into()];
                return Ok(json!({"engine":caps, "services":["password_vault","image_preview","file_checksums","shell_menu"], "machine_format_policy":true, "schema_version":1}));
            }
            let mut caps=engine::capabilities(); caps.write_formats.push("iso (Windows IMAPI2FS)".into());
            caps.read_formats.extend(["egg","alz","ace","asar","br","lz4","aes","lz","uu","uue","xxe","pma","zpaq","daa","isz"].map(str::to_string));
            caps.write_formats.push("lzh (LH7)".into());
            caps.write_formats.extend(["brotli", "lz4", "lz5", "lizard", "zstd"].map(str::to_string));
            caps.read_formats.extend(["brotli", "lz5", "lizard"].map(str::to_string));
            caps.limitations.push("Standalone raw streams contain one regular file and do not support passwords, splitting or archive-specific options; unknown decoded lengths are reported explicitly until extraction".into());
            caps.limitations.push("Raw-stream integrity checks depend on the format; ordinary Brotli has no content checksum, so valid bit changes cannot always be detected".into());
            caps.limitations.retain(|s|!s.contains("ALZ/EGG"));
            caps.limitations.push("RAR creation and some ZIPX codecs are unsupported; LZH writing supports LH7 without passwords/splitting".into());
            caps.limitations.push("ACE adapter has bounded member/total sizes; not every historical ACE variant is validated".into());
            caps.limitations.push("EGG/ALZ adapters are bounded to 100000 entries and 8 GiB total decoded data; ALZ cancellation is checked during decode IO".into());
            caps.limitations.push("Not all historical format variants are covered; ZIP repair is bounded salvage, not guaranteed full recovery".into());
            Ok(json!({"engine":caps,"services":["zip_repair","password_vault","candidate_and_charset_recovery","image_preview","file_checksums","shell_menu"],"schema_version":1}))
        },
        "test" => { test_archive(&r.path, &r.options)?; Ok(json!({"valid":true})) },
        "extract" => {
            if r.dest.is_empty() { return Err("extraction destination required".into()); }
            if r.smart {
                return engine::with_zip_source(&r.path, &r.options, |source| {
                    let entries = list_archive(source, &r.options)?;
                    let destination = crate::smart_extract::destination(&r.path, &r.dest, &entries, &r.options)?;
                    let mut options = r.options.clone();
                    if options.zone_identifier.is_none() {
                        options.zone_identifier = Some(crate::motw::archive_context(std::path::Path::new(&r.path))?);
                    }
                    let files = extract_archive(source, &destination, &options)?;
                    Ok(json!({"files":files,"destination":destination}))
                });
            }
            let count = extract_archive(&r.path, &r.dest, &r.options)?;
            Ok(json!({"files": count, "destination": r.dest}))
        },
        "create" => {
            let format = if r.format.is_empty() { infer_format(&r.path)? } else { r.format.as_str() };
            if format=="lzh" {return Ok(json!({"outputs":crate::legacy::create_lzh(&r.path,&r.inputs,&r.options)?}));}
            if format=="iso" {
                let cancel=r.options.control.as_ref().map(|c|c.cancelled.clone()).unwrap_or_default();
                if r.options.password.is_some() || r.options.volume_bytes.is_some() { return Err("ISO creation does not support passwords or splitting".into()); }
                return Ok(json!({"outputs":[services::create_iso_with_cancel(&r.inputs,&r.path,&cancel)?]}));
            }
            Ok(json!({"outputs": engine::create(&r.path, &r.inputs, format, &r.options)?}))
        },
        "add" | "delete" | "rename" => {
            if !restricted && (crate::installer::recognize(&r.path)?.is_some() || crate::korean::is_supported(&r.path) || crate::legacy::is_supported(&r.path) || crate::stream_formats::is_supported(&r.path) || crate::envelopes::is_supported(&r.path) || crate::rare::supports(&r.path)) {
                return Err("this archive type is read-only; extract its files and create a new archive".into());
            }
            engine::update(&r.path, &r.operation, &r.inputs, &r.options)?;
            Ok(json!({"updated":r.path}))
        },
        "preview" => {
            let bytes = preview_archive(&r.path, &r.name, &r.options)?;
            let mime = image_type(&bytes).ok_or("preview supports PNG, JPEG, GIF, WebP, BMP and AVIF images only")?;
            Ok(json!({"name":r.name,"mime":mime,"data_url": format!("data:{mime};base64,{}", base64(&bytes))}))
        },
        "repair" => Ok(json!(services::repair_zip_with_options(&r.path, &r.dest, r.max_bytes.unwrap_or(512 * 1024 * 1024), r.options.password.as_deref(), r.options.codepage)?)),
        "checksum-files" => crate::checksum::checksum_files(&r.inputs, &r.hashes, &r.options),
        "checksum" => crate::checksum::checksum_selected(&r.path, &r.hashes, &r.options),
        "checksum-algorithms" => Ok(json!({"algorithms": crate::checksum::algorithms(&r.options)?})),
        "comment-get" => Ok(json!({"comment":crate::archive_comment::get(&r.path)?})),
        "comment-set" => {crate::archive_comment::set(&r.path,&r.name,&r.options)?;Ok(json!({"updated":r.path}))},
        "vault-list" => Ok(json!(services::Vault::open(vault_path()?)?.list()?)),
        "vault-put" => {
            let password = r.options.password.as_deref().ok_or("password required")?;
            services::Vault::open(vault_path()?)?.put(&r.name, &r.label, password)?;
            Ok(json!({"saved":r.name}))
        },
        "vault-delete" => { services::Vault::open(vault_path()?)?.remove(&r.name)?; Ok(json!({"removed":r.name})) },
        "recover" => {
            if restricted { return Err("Password recovery is unavailable under NanaZip machine format policy because an engine denial cannot safely be classified as a wrong password".into()); }
            let mut probe=r.options.clone(); probe.password=None;
            match list_archive(&r.path,&probe) {
                Ok(entries) if !entries.iter().any(|e|e.encrypted) => return Err("archive has no encrypted entries".into()),
                Ok(_) => {},
                Err(e) if e.to_lowercase().contains("password") => {},
                Err(e) => return Err(e),
            }
            let cancel = r.options.control.as_ref().map(|c| c.cancelled.clone()).unwrap_or_default();
            let mut check = |password: &str| {
                let mut opts = r.options.clone(); opts.password = Some(password.to_string());
                match test_archive(&r.path, &opts) {
                    Ok(()) => Ok(true),
                    Err(e) if e.to_lowercase().contains("password") || e.to_lowercase().contains("data error") => Ok(false),
                    Err(e) => Err(e),
                }
            };
            let max_attempts = r.max_attempts.unwrap_or(1000);
            if max_attempts > 1_000_000 { return Err("attempt limit exceeds 1000000".into()); }
            let result = if let Some(charset) = &r.charset {
                services::recover_charset_password(charset,r.min_length.unwrap_or(1),r.max_length.unwrap_or(4),max_attempts,&cancel,&mut check)?
            } else {services::recover_password(r.candidates,max_attempts,&cancel,&mut check)?};
            Ok(json!(result))
        },
        "shell-enable" | "shell-disable" => {
            services::shell_menu(r.operation == "shell-enable", &r.path)?;
            Ok(json!({"enabled":r.operation == "shell-enable"}))
        },
        _ => Err(format!("unknown operation {}", r.operation)),
    }
}

fn infer_format(path: &str) -> Result<&str, String> {
    if path.to_lowercase().ends_with(".tar.gz") { return Ok("tgz"); }
    if path.to_lowercase().ends_with(".tar.bz2") { return Ok("tbz2"); }
    if path.to_lowercase().ends_with(".tar.xz") { return Ok("txz"); }
    let lower=path.to_lowercase();
    match std::path::Path::new(&lower).extension().and_then(|v| v.to_str()) {
        Some("zip") => Ok("zip"), Some("7z") => Ok("7z"), Some("tar") => Ok("tar"),
        Some("gz") => Ok("gzip"), Some("bz2") => Ok("bzip2"), Some("xz") => Ok("xz"),
        Some("br" | "brotli") => Ok("brotli"), Some("lz4") => Ok("lz4"), Some("lz5") => Ok("lz5"),
        Some("liz" | "lizard") => Ok("lizard"), Some("zst" | "zstd") => Ok("zstd"),
        Some("lzh") | Some("lha") => Ok("lzh"), Some("wim") => Ok("wim"), Some("iso") => Ok("iso"), Some("zipx") => Ok("zipx"), Some("exe") => Ok("sfx"), Some("tgz") => Ok("tgz"), Some("tbz") | Some("tbz2") => Ok("tbz2"), Some("txz") => Ok("txz"),
        _ => Err("specify format explicitly".into()),
    }
}
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(b"\xff\xd8\xff") { Some("image/jpeg") }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { Some("image/gif") }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { Some("image/webp") }
    else if bytes.starts_with(b"BM") { Some("image/bmp") }
    else if bytes.get(4..12) == Some(b"ftypavif") { Some("image/avif") }
    else { None }
}
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for b in data.chunks(3) {
        let n = (b[0] as u32) << 16 | (b.get(1).copied().unwrap_or(0) as u32) << 8 | b.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[(n >> 18) as usize] as char); out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if b.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if b.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn listing_response_matches_legacy_json_including_key_order() {
        let entry = engine::Entry {
            name: "目录/file".into(), size: u64::MAX, size_known: false, packed_size: 11,
            is_dir: false, crc: "ABCD".into(), encrypted: true, modified: "2026-01-01".into(),
            method: "LZMA2".into(), comment: "quote \" slashes \\ line\n".into(), attributes: "R".into(),
        };
        for entries in [Vec::new(), vec![entry]] {
            let legacy = json!({ "entries": &entries });
            let response = OperationResponse::Listing { entries };
            assert_eq!(serde_json::to_vec_pretty(&response).unwrap(), serde_json::to_vec_pretty(&legacy).unwrap());
            assert_eq!(serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&legacy).unwrap());
        }
    }
    #[test] fn base64_padding() { assert_eq!(base64(b"f"), "Zg=="); assert_eq!(base64(b"fo"), "Zm8="); assert_eq!(base64(b"foo"), "Zm9v"); }
    #[test] fn preview_rejects_active_content() { assert!(image_type(b"<svg onload=alert(1)>").is_none()); assert!(image_type(b"<html>").is_none()); }
}
