//! Bounded archive previews. Text is returned as data, never rendered/executed here.
//! Original images are decoded for validation and exported with their original bytes.
use obz_core::{
    engine::{EngineOptions, Entry},
    operations,
};
use serde::Serialize;
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::{
    collections::{hash_map::RandomState, VecDeque},
    fs::{self, File, OpenOptions},
    hash::BuildHasher,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, SystemTime},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
};

const TEXT_LIMIT: u64 = 2 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;
const CACHE_LIMIT: usize = 64 * 1024 * 1024;
const STAGE_LIMIT: u64 = 256 * 1024 * 1024;
const EXPORT_LIMIT: u64 = 512 * 1024 * 1024;
const MARKER: &[u8] = b"OpenArchive original image preview v1\n";
static CACHE: Mutex<Option<ArchiveCache>> = Mutex::new(None);
static PASSWORD_HASH: OnceLock<RandomState> = OnceLock::new();
static VOLATILE: AtomicU64 = AtomicU64::new(1);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<Option<obz_core::engine::JobControl>> = Mutex::new(None);
static EXPORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(PartialEq, Eq)]
struct Identity {
    path: PathBuf,
    len: u64,
    modified: u64,
    volume: u32,
    index: u64,
    password: u64,
    codepage: Option<u32>,
    volatile: u64,
}
struct ArchiveCache {
    identity: Identity,
    entries: Vec<Entry>,
    stage: Option<tempfile::TempDir>,
    raw: VecDeque<(String, Vec<u8>)>,
    raw_bytes: usize,
}
impl Drop for ArchiveCache {
    fn drop(&mut self) {
        // Clear readonly only on files in this cache's checked manifest, allowing
        // TempDir to remove its own stage without leaving decrypted files behind.
        if let Some(stage) = &self.stage {
            for entry in self.entries.iter().filter(|entry| !entry.is_dir) {
                let Ok(relative) = safe_name(&entry.name) else {
                    continue;
                };
                let file = stage.path().join(relative);
                if no_links(&file).is_err() {
                    continue;
                }
                if let Ok(meta) = fs::metadata(&file) {
                    if meta.is_file() && meta.permissions().readonly() {
                        let mut permissions = meta.permissions();
                        permissions.set_readonly(false);
                        let _ = fs::set_permissions(file, permissions);
                    }
                }
            }
        }
    }
}

fn no_links(path: &Path) -> Result<(), String> {
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(m) if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 => {
                return Err("预览路径不能包含链接或重解析点".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
fn source(path: &str, options: &EngineOptions) -> Result<(File, Identity), String> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with(r"\\.\")
        || path.starts_with(r"\\?\")
    {
        return Err("无效的归档路径".into());
    }
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    no_links(&path)?;
    // Deny concurrent write/delete while listing, decoding and validating this request.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("请选择普通压缩文件".into());
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|e| e.to_string())?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let split = ext == "rar"
        || ext == "exe"
        || ext.bytes().all(|c| c.is_ascii_digit())
        || (ext.len() == 3
            && matches!(ext.as_bytes()[0], b'r' | b'z')
            && ext.as_bytes()[1..].iter().all(u8::is_ascii_digit))
        || path.with_extension("z01").exists()
        || PathBuf::from(format!("{}.001", path.display())).exists()
        || PathBuf::from(format!("{}.002", path.display())).exists();
    let identity = Identity {
        path,
        len: meta.len(),
        modified: meta.last_write_time(),
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        password: PASSWORD_HASH
            .get_or_init(RandomState::new)
            .hash_one(&options.password),
        codepage: options.codepage,
        volatile: if split {
            VOLATILE.fetch_add(1, Ordering::Relaxed)
        } else {
            0
        },
    };
    Ok((file, identity))
}
fn safe_name(name: &str) -> Result<PathBuf, String> {
    // Match the core's permitted leading ./ normalization, then use its shared policy.
    let normalized = name
        .replace('\\', "/")
        .split('/')
        .filter(|part| *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    obz_core::extract::relative_path(&normalized)
}
fn text_name(name: &str) -> bool {
    let leaf = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if [
        "readme",
        "license",
        "notice",
        "changelog",
        "authors",
        "makefile",
        "dockerfile",
        ".gitignore",
        ".editorconfig",
        ".env",
    ]
    .contains(&leaf.as_str())
    {
        return true;
    }
    let ext = leaf.rsplit('.').next().unwrap_or("");
    [
        "txt",
        "md",
        "markdown",
        "log",
        "csv",
        "tsv",
        "json",
        "jsonl",
        "xml",
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "config",
        "properties",
        "html",
        "htm",
        "css",
        "js",
        "mjs",
        "cjs",
        "ts",
        "tsx",
        "jsx",
        "rs",
        "py",
        "java",
        "c",
        "h",
        "cpp",
        "hpp",
        "cs",
        "sql",
        "sh",
        "bat",
        "ps1",
        "svg",
        "srt",
        "ass",
        "vtt",
        "tex",
        "vue",
        "go",
        "kt",
        "dart",
        "rb",
        "php",
    ]
    .contains(&ext)
}
fn image_format(name: &str) -> Result<image::ImageFormat, String> {
    use image::ImageFormat::*;
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png"=>Ok(Png),"jpg"|"jpeg"=>Ok(Jpeg),"gif"=>Ok(Gif),"webp"=>Ok(WebP),"bmp"=>Ok(Bmp),"tif"|"tiff"=>Ok(Tiff),"ico"=>Ok(Ico),"avif"=>Ok(Avif),
        _=>Err("缩略图和外部原图支持 PNG、JPEG、GIF、WebP、BMP、TIFF、ICO；其他格式可尝试工具箱图片预览".into())
    }
}
fn bytes_for(
    path: &str,
    name: &str,
    options: &EngineOptions,
    limit: u64,
    epoch: u64,
) -> Result<Vec<u8>, String> {
    if EPOCH.load(Ordering::SeqCst) != epoch {
        return Err("预览已取消".into());
    }
    let relative = safe_name(name)?;
    let (_source, identity) = source(path, options)?;
    let mut guard = CACHE.lock().map_err(|_| "预览缓存不可用")?;
    if EPOCH.load(Ordering::SeqCst) != epoch {
        return Err("预览已取消".into());
    }
    let control = obz_core::engine::JobControl::new();
    *ACTIVE.lock().map_err(|_| "预览不可用")? = Some(control.clone());
    if EPOCH.load(Ordering::SeqCst) != epoch {
        return Err("预览已取消".into());
    }
    let options = &EngineOptions {
        control: Some(control),
        ..options.clone()
    };
    if guard
        .as_ref()
        .is_none_or(|cache| cache.identity != identity)
    {
        let entries = operations::list_archive(path, options)?;
        if entries.len() > 100_000 {
            return Err("归档条目过多，不能进行预览".into());
        }
        *guard = Some(ArchiveCache {
            identity,
            entries,
            stage: None,
            raw: VecDeque::new(),
            raw_bytes: 0,
        });
    }
    let cache = guard.as_mut().unwrap();
    let entry = cache
        .entries
        .iter()
        .find(|entry| !entry.is_dir && safe_name(&entry.name).is_ok_and(|path| path == relative))
        .ok_or("找不到要预览的文件")?;
    let size = entry.size;
    // Keep the engine spelling for exact-name backends; the UI uses slash paths.
    let engine_name = entry.name.clone();
    if size > limit {
        return Err(format!(
            "此文件超过 {} MiB 预览限制，请解压后查看",
            limit / 1024 / 1024
        ));
    }
    if let Some(index) = cache.raw.iter().position(|(key, _)| key == name) {
        let item = cache.raw.remove(index).unwrap();
        let bytes = item.1.clone();
        cache.raw.push_back(item);
        return Ok(bytes);
    }
    let total = cache
        .entries
        .iter()
        .try_fold(0u64, |sum, e| sum.checked_add(e.size));
    if cache.stage.is_none()
        && cache.identity.volatile == 0
        && cache.entries.len() <= 4096
        && total.is_some_and(|size| size <= STAGE_LIMIT)
    {
        let stage = tempfile::Builder::new()
            .prefix("open-archive-preview-cache-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let mut extract_options = options.clone();
        extract_options.selected.clear();
        extract_options.overwrite = "error".into();
        operations::extract_archive(
            path,
            stage.path().to_str().ok_or("临时目录编码无效")?,
            &extract_options,
        )?;
        if EPOCH.load(Ordering::SeqCst) != epoch {
            return Err("预览已取消".into());
        }
        cache.stage = Some(stage);
    }
    let bytes = if let Some(stage) = &cache.stage {
        let target = stage.path().join(relative);
        no_links(&target)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(target)
            .map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() != size || meta.file_attributes() & 0x400 != 0 {
            return Err("暂存预览文件与清单不一致".into());
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        bytes
    } else {
        operations::preview_archive(path, &engine_name, options)?
    };
    if EPOCH.load(Ordering::SeqCst) != epoch {
        return Err("预览已取消".into());
    }
    if bytes.len() as u64 != size || bytes.len() as u64 > limit {
        return Err("预览大小与清单不一致".into());
    }
    while cache.raw_bytes + bytes.len() > CACHE_LIMIT || cache.raw.len() >= 64 {
        if let Some((_, old)) = cache.raw.pop_front() {
            cache.raw_bytes -= old.len();
        } else {
            break;
        }
    }
    cache.raw_bytes += bytes.len();
    cache.raw.push_back((name.into(), bytes.clone()));
    Ok(bytes)
}
fn decode_text(bytes: &[u8], encoding: Option<&str>) -> Result<(String, String), String> {
    use encoding_rs::{GBK, UTF_16BE, UTF_16LE, WINDOWS_1252};
    let choice = encoding.unwrap_or("auto");
    let (text, label) = if choice == "auto" && bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (
            String::from_utf8(bytes[3..].to_vec()).map_err(|_| "UTF-8 文本编码损坏")?,
            "UTF-8 BOM".to_string(),
        )
    } else if choice == "auto" && bytes.starts_with(&[0xff, 0xfe]) {
        let (s, bad) = UTF_16LE.decode_without_bom_handling(&bytes[2..]);
        if bad {
            return Err("UTF-16LE 文本编码损坏".into());
        }
        (s.into_owned(), "UTF-16LE BOM".into())
    } else if choice == "auto" && bytes.starts_with(&[0xfe, 0xff]) {
        let (s, bad) = UTF_16BE.decode_without_bom_handling(&bytes[2..]);
        if bad {
            return Err("UTF-16BE 文本编码损坏".into());
        }
        (s.into_owned(), "UTF-16BE BOM".into())
    } else if choice == "utf-8" || choice == "auto" && std::str::from_utf8(bytes).is_ok() {
        (
            String::from_utf8(bytes.to_vec()).map_err(|_| "不是有效 UTF-8；请尝试其他编码")?,
            "UTF-8".into(),
        )
    } else {
        let (codec, label) = match choice {
            "auto" => (GBK, "GBK（自动推测）"),
            "gbk" => (GBK, "GBK"),
            "utf-16le" => (UTF_16LE, "UTF-16LE"),
            "utf-16be" => (UTF_16BE, "UTF-16BE"),
            "windows-1252" => (WINDOWS_1252, "Windows-1252"),
            _ => return Err("不支持此文本编码".into()),
        };
        let (s, bad) = codec.decode_without_bom_handling(bytes);
        if bad {
            return Err("文本编码无法可靠识别，请选择其他编码".into());
        }
        (s.trim_start_matches('\u{feff}').to_string(), label.into())
    };
    if text
        .chars()
        .any(|c| c == '\0' || c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
    {
        return Err("文件包含二进制控制内容，不能作为文本预览".into());
    }
    Ok((text, label))
}
fn decoded_image(name: &str, bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let expected = image_format(name)?;
    let actual = image::guess_format(bytes).map_err(|_| "不是受支持的图像文件")?;
    if actual != expected {
        return Err("图像内容与文件扩展名不一致，已拒绝打开".into());
    }
    if actual == image::ImageFormat::Avif {
        return decoded_wic(bytes, "AVIF");
    }
    decoded_portable_image(bytes, actual)
}
fn decoded_portable_image(
    bytes: &[u8],
    actual: image::ImageFormat,
) -> Result<image::DynamicImage, String> {
    let reader = image::ImageReader::with_format(Cursor::new(bytes), actual);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("图像损坏：{e}"))?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 40_000_000 {
        return Err("图像尺寸超过 4000 万像素预览限制".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), actual);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(192 * 1024 * 1024);
    limits.max_image_width = Some(40_000);
    limits.max_image_height = Some(40_000);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| format!("图像无法安全解码：{e}"))
}
fn decoded_wic(bytes: &[u8], label: &str) -> Result<image::DynamicImage, String> {
    use windows::Win32::{Graphics::Imaging::*, System::Com::*, UI::Shell::SHCreateMemStream};
    // Windows codec availability differs between installations. Decode actual pixels,
    // not just the container signature, before handing an image to the default viewer.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
        let stream = SHCreateMemStream(Some(bytes)).ok_or("无法创建有界图像流")?;
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)
            .map_err(|_| {
                format!("系统缺少可用的 {label} 解码器或图像损坏；可使用工具箱内嵌预览")
            })?;
        let frame = decoder.GetFrame(0).map_err(|e| e.to_string())?;
        let (mut width, mut height) = (0, 0);
        frame
            .GetSize(&mut width, &mut height)
            .map_err(|e| e.to_string())?;
        if width == 0
            || height == 0
            || width > 40_000
            || height > 40_000
            || u64::from(width) * u64::from(height) > 40_000_000
        {
            return Err("图像尺寸超过 4000 万像素预览限制".into());
        }
        let converter = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(|e| e.to_string())?;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(|e| format!("{label} 图像无法完整解码：{e}"))?;
        Ok(image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(width, height, pixels).ok_or("图像像素长度不一致")?,
        ))
    }
}
fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize] as char);
        out.push(A[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            A[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
#[derive(Serialize)]
pub struct Preview {
    kind: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
}
fn preview_inner(
    path: &str,
    name: &str,
    options: &EngineOptions,
    kind: &str,
    text_encoding: Option<&str>,
    epoch: u64,
) -> Result<Preview, String> {
    let mut result = Preview {
        kind: kind.into(),
        name: name.into(),
        text: None,
        encoding: None,
        truncated: false,
        data_url: None,
        width: None,
        height: None,
    };
    match kind {
        "text" => {
            if !text_name(name) {
                return Err("此文件类型不支持文本预览".into());
            }
            let bytes = bytes_for(path, name, options, TEXT_LIMIT, epoch)?;
            let (text, encoding) = decode_text(&bytes, text_encoding)?;
            result.text = Some(text);
            result.encoding = Some(encoding);
        }
        "thumbnail" => {
            image_format(name)?;
            let bytes = bytes_for(path, name, options, IMAGE_LIMIT, epoch)?;
            let image = decoded_image(name, &bytes)?;
            result.width = Some(image.width());
            result.height = Some(image.height());
            let mut png = Cursor::new(Vec::new());
            image
                .thumbnail(320, 320)
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            result.data_url = Some(format!("data:image/png;base64,{}", base64(png.get_ref())));
        }
        _ => return Err("不支持的预览方式".into()),
    }
    Ok(result)
}
#[tauri::command]
pub async fn preview_entry(
    path: String,
    name: String,
    password: Option<String>,
    codepage: Option<u32>,
    kind: String,
    text_encoding: Option<String>,
) -> Result<Preview, String> {
    let epoch = EPOCH.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        preview_inner(
            &path,
            &name,
            &EngineOptions {
                password,
                codepage,
                ..Default::default()
            },
            &kind,
            text_encoding.as_deref(),
            epoch,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn clear_preview_cache() -> Result<(), String> {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Some(control) = ACTIVE.lock().map_err(|_| "预览不可用")?.take() {
        control.cancel();
    }
    tauri::async_runtime::spawn_blocking(|| {
        *CACHE.lock().map_err(|_| "预览缓存不可用")? = None;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Only recognizes our flat, marker-bearing image export directories. Never recurses.
fn owned_exports(root: &Path) -> Result<Vec<(PathBuf, Vec<PathBuf>, u64, SystemTime)>, String> {
    no_links(root)?;
    let mut result = Vec::new();
    for item in fs::read_dir(root).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        if !item.file_name().to_string_lossy().starts_with("image-")
            || no_links(&path).is_err()
            || !path.is_dir()
        {
            continue;
        }
        let marker = path.join(".open-archive-preview");
        if no_links(&marker).is_err()
            || fs::metadata(&marker).map(|m| m.len()).ok() != Some(MARKER.len() as u64)
            || fs::read(&marker).ok().as_deref() != Some(MARKER)
        {
            continue;
        }
        let mut files = Vec::new();
        let mut bytes = 0u64;
        let mut valid = true;
        for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            let m = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
            if !m.is_file() || m.file_attributes() & 0x400 != 0 || files.len() >= 2 {
                valid = false;
                break;
            }
            if p != marker && image_format(&p.to_string_lossy()).is_err() {
                valid = false;
                break;
            }
            bytes = bytes.saturating_add(m.len());
            files.push(p);
        }
        if valid {
            result.push((
                path,
                files,
                bytes,
                item.metadata()
                    .map_err(|e| e.to_string())?
                    .modified()
                    .unwrap_or(SystemTime::now()),
            ));
        }
    }
    Ok(result)
}
fn export_original(root: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    no_links(root)?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    no_links(root)?;
    let mut used = 0u64;
    let mut retained_directories = 0usize;
    for (directory, files, size, modified) in owned_exports(root)? {
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            > Duration::from_secs(7 * 86400)
        {
            // A viewer may still hold a file. Failed deletion is left for a later pass.
            let marker = directory.join(".open-archive-preview");
            let mut removed = true;
            for file in files.iter().filter(|p| **p != marker) {
                removed &= no_links(file).is_ok() && fs::remove_file(file).is_ok();
            }
            if removed {
                let _ = fs::remove_file(marker);
                let _ = fs::remove_dir(directory);
            } else {
                used = used.saturating_add(size);
                retained_directories += 1;
            }
        } else {
            used = used.saturating_add(size);
            retained_directories += 1;
        }
    }
    if used.saturating_add(bytes.len() as u64) > EXPORT_LIMIT || retained_directories >= 256 {
        return Err(
            "原图预览临时文件已达 512 MiB 或 256 项，请清理本机 OpenArchive/PreviewFiles 后重试"
                .into(),
        );
    }
    let directory = tempfile::Builder::new()
        .prefix("image-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let leaf = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("图像名称无效")?;
    let path = directory.path().join(leaf);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(1)
        .open(&path)
        .map_err(|e| e.to_string())?;
    output.write_all(bytes).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    fs::write(directory.path().join(".open-archive-preview"), MARKER).map_err(|e| e.to_string())?;
    let _ = directory.keep();
    Ok(path)
}
#[derive(Serialize)]
pub struct OpenedImage {
    path: String,
    opened: bool,
}
#[tauri::command]
pub async fn open_archive_image(
    path: String,
    name: String,
    password: Option<String>,
    codepage: Option<u32>,
) -> Result<OpenedImage, String> {
    let epoch = EPOCH.load(Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        image_format(&name)?;
        let bytes = bytes_for(
            &path,
            &name,
            &EngineOptions {
                password,
                codepage,
                ..Default::default()
            },
            IMAGE_LIMIT,
            epoch,
        )?;
        decoded_image(&name, &bytes)?;
        if EPOCH.load(Ordering::SeqCst) != epoch {
            return Err("预览已取消".into());
        }
        let _lock = EXPORT_LOCK.lock().map_err(|_| "原图预览不可用")?;
        let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不可用")?)
            .join("OpenArchive")
            .join("PreviewFiles");
        let exported = export_original(&root, &name, &bytes)?;
        let wide: Vec<u16> = exported
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let status = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if status.0 as isize <= 32 {
            return Err("系统默认看图程序未能启动，请检查图片文件关联".into());
        }
        Ok(OpenedImage {
            path: exported.to_string_lossy().into_owned(),
            opened: true,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_decoding_preserves_unicode_and_refuses_binary() {
        assert_eq!(
            decode_text("中文 😀".as_bytes(), None).unwrap().0,
            "中文 😀"
        );
        assert_eq!(
            decode_text(&[0xff, 0xfe, 0x2d, 0x4e, 0x87, 0x65], None)
                .unwrap()
                .0,
            "中文"
        );
        assert_eq!(
            decode_text(&[0xd6, 0xd0, 0xce, 0xc4], None).unwrap().0,
            "中文"
        );
        assert!(decode_text(b"a\0b", None).is_err());
        assert!(decode_text(&[0xff], Some("utf-8")).is_err());
        assert_eq!(
            decode_text(b"<script>alert(1)</script>", None).unwrap().0,
            "<script>alert(1)</script>"
        );
    }
    #[test]
    fn image_validation_rejects_active_corrupt_or_mislabeled_data() {
        assert!(decoded_image("fake.png", b"<svg onload='x'>").is_err());
        assert!(decoded_image("fake.jpg", b"\xff\xd8\xff").is_err());
        let image = image::DynamicImage::new_rgb8(2, 3);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(
            decoded_image("real.png", png.get_ref()).unwrap().height(),
            3
        );
        assert!(decoded_image("wrong.jpg", png.get_ref()).is_err());
        assert!(image_format("file.svg").is_err());
    }
    #[test]
    fn ico_rgba_and_corrupt_payload_validation() {
        let icon = |rgba: bool| {
            let image = if rgba {
                image::DynamicImage::new_rgba8(16, 16)
            } else {
                image::DynamicImage::new_rgb8(16, 16)
            };
            let mut png = Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png).unwrap();
            assert_eq!(png.get_ref()[25], if rgba { 6 } else { 2 });
            let mut ico = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
            ico.extend_from_slice(&(png.get_ref().len() as u32).to_le_bytes());
            ico.extend_from_slice(&22u32.to_le_bytes());
            ico.extend_from_slice(png.get_ref());
            ico
        };
        let rgba = icon(true);
        let decoded = decoded_image("rgba.ico", &rgba).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (16, 16));
        assert!(decoded_image("truncated.ico", &rgba[..30]).is_err());
        assert!(decoded_image("disguised.png", &rgba).is_err());
        // A classic 32bpp BGRA DIB icon with the separate AND mask remains supported.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&16i32.to_le_bytes());
        dib.extend_from_slice(&32i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&[0u8; 24]);
        for _ in 0..16 * 16 {
            dib.extend_from_slice(&[30, 20, 10, 255]);
        }
        dib.extend_from_slice(&[0u8; 64]);
        let mut bmp_icon = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
        bmp_icon.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        bmp_icon.extend_from_slice(&22u32.to_le_bytes());
        bmp_icon.extend_from_slice(&dib);
        let bmp = decoded_image("classic.ico", &bmp_icon).unwrap();
        assert_eq!((bmp.width(), bmp.height()), (16, 16));
        assert_eq!(bmp.to_rgba8().get_pixel(0, 0).0, [10, 20, 30, 255]);
        // The pinned portable decoder follows the ICO PNG 32bpp ARGB restriction.
        // A system codec can be more tolerant, but must produce pixels before use.
        assert!(decoded_portable_image(&icon(false), image::ImageFormat::Ico).is_err());
        eprintln!(
            "RGB PNG ICO through WIC: {:?}",
            decoded_wic(&icon(false), "ICO").map(|image| (image.width(), image.height()))
        );
        if let Some(root) = std::env::var_os("OA_PREVIEW_FIXTURE_DIR") {
            for name in ["sample.ico", "sample.avif"] {
                let path = PathBuf::from(&root).join(name);
                if !path.exists() {
                    continue;
                }
                let bytes = fs::read(&path).unwrap();
                eprintln!(
                    "Fixture {name}: {:?}",
                    decoded_image(name, &bytes).map(|image| (image.width(), image.height()))
                );
            }
        }
    }
    #[test]
    fn preview_names_share_core_traversal_policy() {
        for name in [
            "../secret.txt",
            "/root.txt",
            "C:/file.txt",
            "a:b.txt",
            "CON.txt",
            "x/../a.png",
            "file.png ",
        ] {
            assert!(safe_name(name).is_err(), "{name}");
        }
        assert!(safe_name("中文/图像.png").is_ok());
        assert!(text_name("folder/README"));
        assert!(!text_name("file.exe"));
    }
    #[test]
    fn exported_original_bytes_and_unowned_directories_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"validated original bytes";
        let unowned = root.path().join("image-unowned");
        fs::create_dir(&unowned).unwrap();
        fs::write(unowned.join("keep.txt"), b"keep").unwrap();
        let out = export_original(root.path(), "original.png", bytes).unwrap();
        assert_eq!(fs::read(out).unwrap(), bytes);
        assert_eq!(owned_exports(root.path()).unwrap().len(), 1);
        assert_eq!(fs::read(unowned.join("keep.txt")).unwrap(), b"keep");
    }
    #[test]
    fn source_identity_changes_with_password_and_content() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.zip");
        fs::write(&path, b"one").unwrap();
        let name = path.to_str().unwrap();
        let (handle, a) = source(name, &EngineOptions::default()).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(handle);
        let (_, b) = source(
            name,
            &EngineOptions {
                password: Some("different".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(a != b);
        fs::write(&path, b"longer").unwrap();
        let (_, c) = source(name, &EngineOptions::default()).unwrap();
        assert!(a != c);
    }
    #[test]
    fn archive_cache_reuses_safe_stage_and_invalidates_after_replacement() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("one.txt"), b"first").unwrap();
        fs::write(input.join("two.txt"), b"second").unwrap();
        let archive = root.path().join("sample.zip");
        let path = archive.to_str().unwrap();
        obz_core::engine::create(
            path,
            &[input.to_string_lossy().into_owned()],
            "zip",
            &EngineOptions::default(),
        )
        .unwrap();
        let names = operations::list_archive(path, &EngineOptions::default()).unwrap();
        let one = &names
            .iter()
            .find(|e| e.name.ends_with("one.txt"))
            .unwrap()
            .name;
        let two = &names
            .iter()
            .find(|e| e.name.ends_with("two.txt"))
            .unwrap()
            .name;
        let epoch = EPOCH.load(Ordering::SeqCst);
        let options = EngineOptions::default();
        assert_eq!(
            bytes_for(path, &one.replace('\\', "/"), &options, TEXT_LIMIT, epoch).unwrap(),
            b"first"
        );
        let first_stage = CACHE
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stage
            .as_ref()
            .unwrap()
            .path()
            .to_path_buf();
        assert_eq!(
            bytes_for(path, &two.replace('\\', "/"), &options, TEXT_LIMIT, epoch).unwrap(),
            b"second"
        );
        assert_eq!(
            CACHE
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .stage
                .as_ref()
                .unwrap()
                .path(),
            first_stage
        );
        let cached_file = first_stage.join(safe_name(one).unwrap());
        let mut permissions = fs::metadata(&cached_file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&cached_file, permissions).unwrap();
        fs::remove_file(&archive).unwrap();
        fs::write(input.join("one.txt"), b"changed content").unwrap();
        obz_core::engine::create(
            path,
            &[input.to_string_lossy().into_owned()],
            "zip",
            &options,
        )
        .unwrap();
        assert_eq!(
            bytes_for(path, one, &options, TEXT_LIMIT, epoch).unwrap(),
            b"changed content"
        );
        assert!(!first_stage.exists());
        EPOCH.fetch_add(1, Ordering::SeqCst);
        assert!(bytes_for(path, one, &options, TEXT_LIMIT, epoch).is_err());
        *CACHE.lock().unwrap() = None;
    }
}
