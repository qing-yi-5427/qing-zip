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
    #[cfg(windows)]
    pub link_kind: Option<engine::file_workspace::LinkKind>,
    pub wsl_target: Option<String>,
    #[cfg(windows)]
    pub link_receipt: Option<engine::file_workspace::LinkReceipt>,
    pub link_target: Option<String>,
    pub stream_revision: Option<String>,
    #[cfg(windows)]
    pub stream_exports: Vec<engine::file_workspace::StreamExportItem>,
    pub label: String,
    pub password_id: Option<String>,
    pub candidates: Vec<String>,
    pub charset: Option<String>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub max_attempts: Option<usize>,
    pub max_bytes: Option<u64>,
    pub smart: bool,
    pub detect_header_encryption: bool,
    pub allow_unencrypted_projection: bool,
    pub hashes: Vec<String>,
    pub benchmark: Option<engine::benchmark::Options>,
    #[cfg(all(windows,target_pointer_width="64"))]
    pub deep: Option<engine::deep_extract::Settings>,
    pub deep_session: Option<u64>,
    pub deep_node: Option<usize>,
    pub deep_skip: bool,
    #[cfg(windows)]
    pub source_archive: Option<engine::archive_export::Source>,
}

pub fn vault_path() -> Result<PathBuf, String> {
    services::Vault::default_path()
}

pub fn list_archive(path:&str,opts:&EngineOptions)->Result<Vec<engine::Entry>,String>{
    let result=list_archive_once(path,opts);
    #[cfg(windows)]
    if result.is_err() && engine::recover_interrupted_edit(std::path::Path::new(path))? {
        return list_archive_once(path,opts);
    }
    result
}
pub fn list_archive_once(path:&str,opts:&EngineOptions)->Result<Vec<engine::Entry>,String>{
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
    extract_archive_inner(path,dest,opts,false,false)
}
#[cfg(windows)]
pub(crate) fn extract_archive_for_export(path:&str,dest:&str,opts:&EngineOptions)->Result<usize,String>{
    extract_archive_inner(path,dest,opts,true,false)
}
fn extract_archive_inner(path:&str,dest:&str,opts:&EngineOptions,export:bool,allow_unencrypted_projection:bool)->Result<usize,String>{
    let policy = crate::machine_policy::scope();
    if policy.restricted() { return engine::extract_with_projection(path, dest, opts, allow_unencrypted_projection); }
    let mut inherited = opts.clone();
    if inherited.zone_identifier.is_none() { inherited.zone_identifier = Some(crate::motw::archive_context(std::path::Path::new(path))?); }
    let opts = &inherited;
    if crate::rare::supports(path){return crate::rare::extract(path,dest,opts);}
    if crate::envelopes::is_supported(path){return crate::envelopes::extract(path,dest,opts);}
    if crate::stream_formats::is_supported(path) {
        return if export {
            crate::stream_formats::extract_for_export(path, dest, opts)
        } else {
            crate::stream_formats::extract(path, dest, opts)
        };
    }
    if let Some(data)=crate::installer::recognize(path)? {
        return if export {
            crate::installer::extract_for_export(&data, dest, opts)
        } else {
            crate::installer::extract(&data, dest, opts)
        };
    }
    let korean = crate::korean::is_supported(path);
    if korean || crate::legacy::is_supported(path) {
        // These decoders need a complete stage to prove solid/split members.
        let mut complete;
        let options = if export {
            complete = inherited.clone();
            complete.selected.clear();
            &complete
        } else {
            opts
        };
        return if korean {
            crate::korean::extract(path, dest, options)
        } else {
            crate::legacy::extract(path, dest, options)
        };
    }
    engine::extract_with_projection(path,dest,opts,allow_unencrypted_projection)
}
pub fn preview_archive(path:&str,name:&str,opts:&EngineOptions)->Result<Vec<u8>,String>{
    preview_archive_limited(path, name, opts, 32 * 1024 * 1024)
}

pub fn preview_archive_limited(path: &str, name: &str, opts: &EngineOptions, limit: u64) -> Result<Vec<u8>, String> {
    if limit == 0 || limit > 32 * 1024 * 1024 { return Err("invalid archive preview limit".into()); }
    let policy = crate::machine_policy::scope();
    let bytes = if policy.restricted() {
        engine::preview_limited(path, name, opts, limit)
    } else if crate::rare::supports(path) {
        crate::rare::preview(path, name, opts)
    } else if crate::envelopes::is_supported(path) {
        crate::envelopes::preview(path, name, opts)
    } else if crate::stream_formats::is_supported(path) {
        crate::stream_formats::preview(path, name, opts)
    } else if let Some(data) = crate::installer::recognize(path)? {
        crate::installer::preview(&data, name, opts)
    } else if crate::korean::is_supported(path) {
        crate::korean::preview(path, name, opts)
    } else if crate::legacy::is_supported(path) {
        crate::legacy::preview(path, name, opts)
    } else {
        engine::preview_limited(path, name, opts, limit)
    }?;
    if bytes.len() as u64 > limit { return Err("preview exceeds size limit".into()); }
    policy.validate_output()?;
    Ok(bytes)
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OperationResponse {
    Listing {
        #[serde(serialize_with = "serialize_listing_entries")]
        entries: Vec<engine::Entry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        header_encrypted: Option<bool>,
    },
    Json(Value),
}

fn serialize_listing_entries<S: serde::Serializer>(entries: &[engine::Entry], serializer: S) -> Result<S::Ok, S::Error> {
    // The public CLI and desktop wire format formerly serialized Value maps
    // in lexical key order. Keep that order while borrowing each entry.
    #[derive(Serialize)]
    struct Row<'a> {
        accessed: &'a str, attributes: &'a str, comment: &'a str, crc: &'a str, created: &'a str, encrypted: bool,
        is_dir: bool, method: &'a str, modified: &'a str, name: &'a str,
        packed_size: u64, size: u64, size_known: bool,
    }
    serializer.collect_seq(entries.iter().map(|entry| {
        let engine::Entry { accessed, attributes, comment, crc, created, encrypted, is_dir, method, modified, name, packed_size, size, size_known } = entry;
        Row { accessed, attributes, comment, crc, created, encrypted: *encrypted, is_dir: *is_dir, method,
              modified, name, packed_size: *packed_size, size: *size, size_known: *size_known }
    }))
}

/// Compatibility entry point for callers that inspect or mutate the JSON tree.
pub fn execute(r: Request) -> Result<Value, String> {
    match execute_request(r)? {
        OperationResponse::Listing { entries, header_encrypted } => {
            let mut result = json!({ "entries": entries });
            if let Some(encrypted) = header_encrypted {
                result["header_encrypted"] = json!(encrypted);
            }
            Ok(result)
        },
        OperationResponse::Json(value) => Ok(value),
    }
}

/// No shell commands or arbitrary application execution are accepted here.
pub fn execute_request(mut r: Request) -> Result<OperationResponse, String> {
    if r.options.preserve_input_atime && !matches!(r.operation.as_str(),"add"|"archive-add"|"archive-move"|"create") {
        return Err("此操作尚未覆盖输入访问时间保护，未执行操作".into());
    }
    if r.operation != "add" && r.options.update_mode != engine::UpdateMode::Add {
        return Err("更新模式仅适用于添加文件".into());
    }
    let archive_operation = matches!(r.operation.as_str(), "capabilities" | "archive-properties" | "list" | "test" | "extract" | "create" | "add" | "archive-add" | "archive-move" | "archive-copy" | "archive-export" | "delete" | "rename" | "preview" | "repair" | "comment-get" | "comment-set" | "recover");
    let policy = archive_operation.then(crate::machine_policy::scope);
    let restricted = policy.as_ref().is_some_and(|p| p.restricted());
    if let Some(id) = &r.password_id {
        if r.options.password.is_some() { return Err("choose password or password_id, not both".into()); }
        r.options.password = Some(services::Vault::open(vault_path()?)?.get(id)?);
    }
    if r.operation == "list" {
        let header_encrypted = r.detect_header_encryption.then_some(false);
        if r.detect_header_encryption && r.options.password.is_some() {
            let mut no_password = r.options.clone();
            no_password.password = None;
            match list_archive(&r.path, &no_password) {
                Ok(entries) => return Ok(OperationResponse::Listing { entries, header_encrypted }),
                Err(error) => {
                    let error_lower = error.to_ascii_lowercase();
                    if !["password", "encrypted archive"].iter().any(|term| error_lower.contains(term)) {
                        return Err(error);
                    }
                }
            }
            let entries = list_archive(&r.path, &r.options)?;
            return Ok(OperationResponse::Listing { entries, header_encrypted: Some(true) });
        }
        return Ok(OperationResponse::Listing { entries: list_archive(&r.path, &r.options)?, header_encrypted });
    }
    #[cfg(windows)]
    let atime=if r.options.preserve_input_atime && matches!(r.operation.as_str(),"add"|"archive-add"|"archive-move"|"create") {
        let report=std::sync::Arc::new(std::sync::Mutex::new(crate::source_access::Report::default()));r.options.input_atime_report=Some(report.clone());Some(report)
    }else{None};
    let mut result=execute_json(r, restricted)?;
    #[cfg(windows)]
    if let Some(report)=atime{let report=report.lock().unwrap_or_else(|e|e.into_inner());result["input_atime"]=serde_json::to_value(&*report).map_err(|e|e.to_string())?;result["input_atime"]["summary"]=report.summary().into();}
    Ok(OperationResponse::Json(result))
}

fn execute_json(r: Request, restricted: bool) -> Result<Value, String> {
    match r.operation.as_str() {
        "benchmark" => serde_json::to_value(engine::benchmark::execute(r.benchmark.unwrap_or_default(), &r.options)?).map_err(|e|e.to_string()),
        "archive-properties" => Ok(json!(engine::archive_properties::read(&r.path, &r.options)?)),
        #[cfg(windows)]
        "fs-streams" => Ok(serde_json::to_value(engine::file_workspace::list_streams(&r.path)?).map_err(|e|e.to_string())?),
        #[cfg(windows)]
        "fs-export-streams" => Ok(serde_json::to_value(engine::file_workspace::export_streams(&r.path,r.stream_revision.as_deref().ok_or("请刷新附加数据流后重试")?,&r.stream_exports,&r.dest,&r.options)?).map_err(|e|e.to_string())?),
        #[cfg(windows)]
        "fs-list" => Ok(serde_json::to_value(engine::file_workspace::browse(&r.path)?).map_err(|e|e.to_string())?),
        #[cfg(windows)]
        "fs-create-file" | "fs-create-directory" => Ok(serde_json::to_value(engine::file_workspace::create_item(&r.path,&r.name,r.operation=="fs-create-directory",&r.options)?).map_err(|e|e.to_string())?),
        #[cfg(windows)]
        "fs-create-link" => {
            let kind=r.link_kind.ok_or("请选择链接类型")?;
            let target=if kind==engine::file_workspace::LinkKind::Wsl{
                if !r.path.is_empty(){return Err("WSL 目标请使用独立的 wsl_target 字段".into());}
                r.wsl_target.as_deref().ok_or("请输入 WSL 目标路径")?
            }else{if r.wsl_target.is_some(){return Err("只有 WSL 链接接受 wsl_target".into());}&r.path};
            Ok(serde_json::to_value(engine::file_workspace::create_link(target,&r.dest,&r.name,kind,&r.options)?).map_err(|e|e.to_string())?)
        },
        #[cfg(windows)]
        "fs-inspect-link"=>Ok(serde_json::to_value(engine::file_workspace::inspect_link(&r.path,&r.options)?).map_err(|e|e.to_string())?),
        #[cfg(windows)]
        "fs-edit-link"|"fs-clear-link-data"=>{
            if r.link_kind.is_some()||r.wsl_target.is_some(){return Err("已有链接操作由原始链接类型决定，请先重新读取链接".into());}
            let receipt=r.link_receipt.as_ref().ok_or("请先读取链接并提供原始链接凭据")?;
            let target=if r.operation=="fs-edit-link"{Some(r.link_target.as_deref().filter(|s|!s.is_empty()).ok_or("目标不能为空；如需清除，请使用独立的清除链接数据操作")?)}else{
                if r.link_target.is_some(){return Err("清除链接数据操作不接受目标路径".into());}None
            };
            Ok(serde_json::to_value(engine::file_workspace::edit_link(&r.path,receipt,target,&r.options)?).map_err(|e|e.to_string())?)
        },
        #[cfg(windows)]
        "fs-copy" | "fs-move" => Ok(serde_json::to_value(engine::file_workspace::transfer(&r.inputs, &r.dest, r.operation == "fs-move", &r.options)?).map_err(|e|e.to_string())?),
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
            let projection = r.allow_unencrypted_projection && r.options.password.is_none();
            if r.smart {
                let archive = std::path::Path::new(&r.path);
                if archive.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("zip") || extension.eq_ignore_ascii_case("rar"))
                    && archive.is_file()
                    && (restricted || crate::installer::recognize(&r.path)?.is_none())
                {
                    let (files, destination) = engine::extract_smart_engine(&r.path, &r.dest, &r.options, projection)?;
                    return Ok(json!({"files":files,"destination":destination}));
                }
                return engine::with_zip_source(&r.path, &r.options, |source| {
                    let mut options = r.options.clone();
                    let password_evidence = options.extraction_password_use.take();
                    let entries = list_archive(source, &options)?;
                    let destination = crate::smart_extract::destination(&r.path, &r.dest, &entries, &options)?;
                    options.extraction_password_use = password_evidence;
                    if options.zone_identifier.is_none() {
                        options.zone_identifier = Some(crate::motw::archive_context(std::path::Path::new(&r.path))?);
                    }
                    let files = extract_archive_inner(source, &destination, &options, false, projection)?;
                    Ok(json!({"files":files,"destination":destination}))
                });
            }
            let count = extract_archive_inner(&r.path, &r.dest, &r.options, false, projection)?;
            Ok(json!({"files": count, "destination": r.dest}))
        },
        "create" => {
            let format = if r.format.is_empty() { infer_format(&r.path)? } else { r.format.as_str() };
            if r.options.preserve_input_atime && !matches!(format,"zip"|"7z"|"sfx"){return Err("此格式尚未覆盖输入访问时间保护，未创建输出".into());}
            engine::compression_times::validate(format, &r.options)?;
            engine::compression_parameters::switches(format, &r.options)?;
            if format=="lzh" {return Ok(json!({"outputs":crate::legacy::create_lzh(&r.path,&r.inputs,&r.options)?}));}
            if format=="iso" {
                let cancel=r.options.control.as_ref().map(|c|c.cancelled.clone()).unwrap_or_default();
                if r.options.password.is_some() || r.options.volume_bytes.is_some() { return Err("ISO creation does not support passwords or splitting".into()); }
                return Ok(json!({"outputs":[services::create_iso_with_cancel(&r.inputs,&r.path,&cancel)?]}));
            }
            Ok(json!({"outputs": engine::create(&r.path, &r.inputs, format, &r.options)?}))
        },
        #[cfg(windows)]
        "archive-export" => {
            let source = r.source_archive.as_ref().ok_or("缺少源压缩包参数")?;
            Ok(serde_json::to_value(engine::archive_export_local::copy(&r.path,source,&r.dest,&r.options)?).map_err(|e|e.to_string())?)
        },
        "add" | "archive-add" | "archive-move" | "archive-copy" | "delete" | "rename" => {
            if !restricted && (crate::installer::recognize(&r.path)?.is_some() || crate::korean::is_supported(&r.path) || crate::legacy::is_supported(&r.path) || crate::stream_formats::is_supported(&r.path) || crate::envelopes::is_supported(&r.path) || crate::rare::supports(&r.path)) {
                return Err("this archive type is read-only; extract its files and create a new archive".into());
            }
            if r.operation == "archive-copy" {
                #[cfg(windows)]
                engine::archive_export::copy(r.source_archive.as_ref().ok_or("缺少源压缩包参数")?, &r.path, &r.dest, &r.options)?;
                #[cfg(not(windows))]
                return Err("文件管理复制仅支持 Windows".into());
            } else if r.operation == "archive-move" {
                #[cfg(windows)]
                return serde_json::to_value(engine::archive_move(&r.path,&r.inputs,&r.dest,&r.options)?).map_err(|e|e.to_string());
                #[cfg(not(windows))]
                return Err("文件管理移动仅支持 Windows".into());
            } else if r.operation == "archive-add" {
                #[cfg(windows)]
                engine::archive_add(&r.path, &r.inputs, &r.dest, &r.options)?;
                #[cfg(not(windows))]
                return Err("文件管理添加仅支持 Windows".into());
            } else if r.operation == "add" && !r.dest.is_empty() {
                #[cfg(windows)]
                engine::update_in_folder(&r.path, &r.inputs, &r.dest, &r.options)?;
                #[cfg(not(windows))]
                return Err("指定压缩包内的目标目录仅支持 Windows".into());
            } else {
                engine::update(&r.path, &r.operation, &r.inputs, &r.options)?;
            }
            Ok(json!({"updated":r.path}))
        },
        "preview" => {
            let bytes = preview_archive(&r.path, &r.name, &r.options)?;
            let mime = image_type(&bytes).ok_or("preview supports PNG, JPEG, GIF, WebP, BMP and AVIF images only")?;
            Ok(json!({"name":r.name,"mime":mime,"data_url": format!("data:{mime};base64,{}", base64(&bytes))}))
        },
        #[cfg(windows)]
        "file-split" => engine::file_parts::split(&r.path, &r.dest, r.max_bytes.ok_or("请填写分块大小")?, &r.options),
        #[cfg(windows)]
        "file-combine" => engine::file_parts::combine(&r.path, &r.dest, r.max_bytes, &r.options),
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
    #[cfg(windows)]
    #[test] fn local_link_request_contract_and_no_replace(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("source");std::fs::write(&source,b"keep").unwrap();
        let base=json!({"operation":"fs-create-link","path":source,"dest":root.path(),"name":"link","link_kind":"hard","options":{"overwrite":"error"}});
        for bad in [json!("Wsl"),json!("Hard"),json!(3),json!({})] {let mut request=base.clone();request["link_kind"]=bad;assert!(serde_json::from_value::<Request>(request).is_err());}
        for field in ["path","dest","name","link_kind"] {
            let mut request=base.clone();request.as_object_mut().unwrap().remove(field);
            assert!(execute(serde_json::from_value(request).unwrap()).is_err(),"{field}");
        }
        let mut overwrite=base.clone();overwrite["options"]["overwrite"]=json!("overwrite");
        assert!(execute(serde_json::from_value(overwrite).unwrap()).is_err());assert!(!root.path().join("link").exists());
        let result=execute(serde_json::from_value(base.clone()).unwrap()).unwrap();assert_eq!(result["kind"],"hard");
        assert!(execute(serde_json::from_value(base).unwrap()).is_err());assert_eq!(std::fs::read(source).unwrap(),b"keep");
        let target=root.path().join("directory");std::fs::create_dir(&target).unwrap();std::fs::write(target.join("child"),b"junction bytes").unwrap();
        let junction=json!({"operation":"fs-create-link","path":target,"dest":root.path(),"name":"junction","link_kind":"junction","options":{"overwrite":"error"}});
        let result=execute(serde_json::from_value(junction.clone()).unwrap()).unwrap();assert_eq!(result["kind"],"junction");
        assert_eq!(std::fs::read(root.path().join("junction/child")).unwrap(),b"junction bytes");assert!(execute(serde_json::from_value(junction).unwrap()).is_err());
        std::fs::remove_dir(root.path().join("junction")).unwrap();assert_eq!(std::fs::read(target.join("child")).unwrap(),b"junction bytes");
        let wsl=json!({"operation":"fs-create-link","dest":root.path(),"name":"wsl","link_kind":"wsl","wsl_target":"../用户/target","options":{"overwrite":"error"}});
        for change in ["missing","path","cross-kind"]{
            let mut invalid=wsl.clone();match change{"missing"=>{invalid.as_object_mut().unwrap().remove("wsl_target");},"path"=>invalid["path"]=json!("C:/source"),_=>invalid["link_kind"]=json!("hard")};
            assert!(execute(serde_json::from_value(invalid).unwrap()).is_err());assert!(std::fs::symlink_metadata(root.path().join("wsl")).is_err());
        }
        let result=execute(serde_json::from_value(wsl.clone()).unwrap()).unwrap();assert_eq!(result["kind"],"wsl");assert_eq!(result["target"],"../用户/target");
        assert!(execute(serde_json::from_value(wsl).unwrap()).is_err());std::fs::remove_file(root.path().join("wsl")).unwrap();
    }
    #[cfg(windows)]
    #[test] fn existing_link_request_inspect_edit_clear_contract(){
        let root=tempfile::tempdir().unwrap();let path=root.path().join("link");let run=|v:Value|execute(serde_json::from_value(v).unwrap());
        run(json!({"operation":"fs-create-link","dest":root.path(),"name":"link","link_kind":"wsl","wsl_target":"../old"})).unwrap();
        let inspect=json!({"operation":"fs-inspect-link","path":path});let first=run(inspect.clone()).unwrap();assert_eq!(first["target"],"../old");assert_eq!(first["can_edit"],true);assert!(first["receipt"]["file_id"].is_string());
        let edit=json!({"operation":"fs-edit-link","path":path,"link_receipt":first["receipt"],"link_target":"/new/用户"});
        for field in ["link_receipt","link_target"]{let mut invalid=edit.clone();invalid.as_object_mut().unwrap().remove(field);assert!(run(invalid).is_err());}
        let mut empty=edit.clone();empty["link_target"]=json!("");assert!(run(empty).is_err());assert_eq!(run(inspect.clone()).unwrap()["receipt"],first["receipt"]);
        run(edit.clone()).unwrap();assert!(run(edit).is_err());let after=run(inspect.clone()).unwrap();assert_eq!(after["target"],"/new/用户");
        let clear=json!({"operation":"fs-clear-link-data","path":path,"link_receipt":after["receipt"]});let mut ambiguous=clear.clone();ambiguous["link_target"]=json!("");assert!(run(ambiguous).is_err());
        assert_eq!(run(clear).unwrap()["removed"],true);assert!(std::fs::symlink_metadata(&path).unwrap().is_file());assert!(run(inspect).is_err());
    }
    #[test]
    fn preserve_input_atime_unsupported_operations_fail_before_output_or_source_reads(){
        let temp=tempfile::tempdir().unwrap();let source=temp.path().join("source.txt");std::fs::write(&source,b"original").unwrap();let output=temp.path().join("not-created/archive.zip");
        for operation in ["list","test","extract","archive-copy","archive-export","delete","rename","hash"]{
            let r:Request=serde_json::from_value(json!({"operation":operation,"path":output,"dest":temp.path().join("not-created"),"inputs":[source],"options":{"preserve_input_atime":true}})).unwrap();
            let error=execute_request(r).err().unwrap();assert!(error.contains("输入访问时间保护"),"{operation}: {error}");assert!(!output.parent().unwrap().exists());assert_eq!(std::fs::read(&source).unwrap(),b"original");
        }
        let options=engine::EngineOptions{preserve_input_atime:true,..Default::default()};
        for format in ["tar","gzip","iso","lzh","zipx"]{let error=engine::create(output.to_str().unwrap(),&[source.display().to_string()],format,&options).unwrap_err();assert!(error.contains("输入访问时间保护"));assert!(!output.parent().unwrap().exists());}
        let plain=temp.path().join("ordinary.zip");engine::create(plain.to_str().unwrap(),&[source.display().to_string()],"zip",&engine::EngineOptions::default()).unwrap();assert!(plain.is_file());engine::test(plain.to_str().unwrap(),&engine::EngineOptions::default()).unwrap();
    }
    #[test]
    fn listing_response_matches_legacy_json_including_key_order() {
        let entry = engine::Entry {
            created: "2025-01-02 03:04:05Z".into(), accessed: "2026-02-03 04:05:06Z".into(),
            name: "目录/file".into(), size: u64::MAX, size_known: false, packed_size: 11,
            is_dir: false, crc: "ABCD".into(), encrypted: true, modified: "2026-01-01".into(),
            method: "LZMA2".into(), comment: "quote \" slashes \\ line\n".into(), attributes: "R".into(),
        };
        for entries in [Vec::new(), vec![entry]] {
            let legacy = json!({ "entries": &entries });
            let response = OperationResponse::Listing { entries, header_encrypted: None };
            assert_eq!(serde_json::to_vec_pretty(&response).unwrap(), serde_json::to_vec_pretty(&legacy).unwrap());
            assert_eq!(serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&legacy).unwrap());
        }
    }
    #[test]
    fn mixed_zip_plaintext_scope_is_opt_in_for_both_extraction_modes() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("mixed.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("public/readme.txt", options).unwrap();
        writer.write_all(b"public").unwrap();
        writer.start_file("private/secret.txt", options.with_aes_encryption(zip::AesMode::Aes256, "fixture-pass")).unwrap();
        writer.write_all(b"private").unwrap();
        writer.finish().unwrap();
        let source = std::fs::read(&archive).unwrap();
        for smart in [false, true] {
            for (scope, enabled, success) in [
                ("public/", false, false),
                ("public/", true, true),
                ("private/", true, false),
            ] {
                let target = root.path().join(format!("out-{smart}-{}-{enabled}", scope.trim_end_matches('/')));
                let result = execute(Request {
                    operation: "extract".into(),
                    path: archive.display().to_string(),
                    dest: target.display().to_string(),
                    smart,
                    allow_unencrypted_projection: enabled,
                    options: EngineOptions { selected: vec![scope.into()], ..Default::default() },
                    ..Default::default()
                });
                if success {
                    let result = result.unwrap();
                    let output = std::path::Path::new(result["destination"].as_str().unwrap());
                    assert_eq!(std::fs::read(output.join("public/readme.txt")).unwrap(), b"public");
                    assert!(!output.join("private").exists());
                } else {
                    assert!(result.unwrap_err().to_ascii_lowercase().contains("password"));
                    assert!(!target.join("public/readme.txt").exists());
                }
            }
        }
        assert_eq!(std::fs::read(&archive).unwrap(), source);
    }
    #[test]
    fn extraction_password_evidence_requires_encrypted_output_or_authenticated_headers() {
        use std::{io::Write, sync::Arc};
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("mixed.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("public.txt", options).unwrap();
        writer.write_all(b"public").unwrap();
        writer.start_file("secret.txt", options.with_aes_encryption(zip::AesMode::Aes256, "fixture-pass")).unwrap();
        writer.write_all(b"secret").unwrap();
        writer.finish().unwrap();
        for mode in ["full", "smart", "selected", "public", "skip-encrypted", "skip-public", "wrong", "cancel"] {
            let dest = root.path().join(mode);
            std::fs::create_dir(&dest).unwrap();
            if mode == "skip-encrypted" || mode == "skip-public" {
                let name = if mode == "skip-encrypted" { "secret.txt" } else { "public.txt" };
                std::fs::write(dest.join(name), b"existing").unwrap();
            }
            let evidence = Arc::new(engine::ExtractionPasswordUse::default());
            let control = engine::JobControl::new();
            if mode == "cancel" {
                control.cancel();
            }
            let result = execute(Request {
                operation: "extract".into(),
                path: archive.display().to_string(),
                dest: dest.display().to_string(),
                smart: mode == "smart",
                allow_unencrypted_projection: mode == "public",
                options: EngineOptions {
                    password: Some(if mode == "wrong" { "bad" } else { "fixture-pass" }.into()),
                    overwrite: if mode.starts_with("skip-") { "skip" } else { "error" }.into(),
                    selected: match mode {
                        "selected" => vec!["secret.txt".into()],
                        "public" => vec!["public.txt".into()],
                        _ => vec![],
                    },
                    control: Some(control),
                    extraction_password_use: Some(evidence.clone()),
                    ..Default::default()
                },
                ..Default::default()
            });
            if matches!(mode, "wrong" | "cancel") {
                assert!(result.is_err(), "{mode}");
            } else {
                let count = result.unwrap_or_else(|error| panic!("{mode}: {error}"))["files"].as_u64().unwrap() as usize;
                assert_eq!(evidence.verified(count), matches!(mode, "full" | "smart" | "selected" | "skip-public"), "{mode}");
            }
        }

        let source = root.path().join("payload.txt");
        std::fs::write(&source, b"one file").unwrap();
        let empty = root.path().join("empty.txt");
        std::fs::write(&empty, []).unwrap();
        for (name, source, encrypted, header_only) in [
            ("plain.7z", &source, false, false),
            ("headers.7z", &source, true, false),
            ("header-only.7z", &empty, true, true),
        ] {
            let archive = root.path().join(name);
            let path = archive.to_str().unwrap();
            engine::create(path, &[source.display().to_string()], "7z", &EngineOptions {
                password: encrypted.then(|| "fixture-pass".into()),
                header_encryption: encrypted.then_some(true),
                ..Default::default()
            }).unwrap();
            if header_only {
                let entries = list_archive(path, &EngineOptions {
                    password: Some("fixture-pass".into()),
                    ..Default::default()
                }).unwrap();
                assert_eq!(entries.len(), 1);
                assert!(!entries[0].encrypted);
            }
            let evidence = Arc::new(engine::ExtractionPasswordUse::default());
            let count = extract_archive(path, root.path().join(format!("out-{name}")).to_str().unwrap(), &EngineOptions {
                password: Some("fixture-pass".into()),
                extraction_password_use: Some(evidence.clone()),
                ..Default::default()
            }).unwrap();
            assert_eq!(evidence.verified(count), encrypted);
        }
    }
    #[test]
    fn listing_distinguishes_encrypted_directory_from_supplied_password() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("payload.txt");
        std::fs::write(&source, b"data").unwrap();
        for (archive_name, encrypted) in [("plain.zip", false), ("locked.7z", true)] {
            let archive = root.path().join(archive_name);
            let path = archive.display().to_string();
            let password = Some("fixture-pass".to_string());
            let create_options = EngineOptions {
                password: if encrypted { password.clone() } else { None },
                header_encryption: encrypted.then_some(true),
                ..Default::default()
            };
            engine::create(&path, &[source.display().to_string()], if encrypted { "7z" } else { "zip" }, &create_options).unwrap();
            let result = execute(Request {
                operation: "list".into(),
                path,
                detect_header_encryption: true,
                options: EngineOptions { password, ..Default::default() },
                ..Default::default()
            }).unwrap();
            assert_eq!(result["header_encrypted"], encrypted);
            assert_eq!(result["entries"][0]["name"], "payload.txt");
        }
    }
    #[test]
    fn create_rejects_unsupported_time_options_before_format_dispatch() {
        for format in ["iso", "lzh", "gzip", "zipx"] {
            let request: Request = serde_json::from_value(json!({
                "operation":"create", "path":"unused-output", "format":format,
                "inputs":[], "options":{"store_ctime":true}
            })).unwrap();
            let error=execute_json(request,false).unwrap_err();
            assert!(error.contains("时间保存选项"), "{format}: {error}");
        }
    }
    #[test] fn base64_padding() { assert_eq!(base64(b"f"), "Zg=="); assert_eq!(base64(b"fo"), "Zm8="); assert_eq!(base64(b"foo"), "Zm9v"); }
    #[test] fn preview_rejects_active_content() { assert!(image_type(b"<svg onload=alert(1)>").is_none()); assert!(image_type(b"<html>").is_none()); }
}
