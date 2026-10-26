//! qing-zip desktop app (Tauri v2): archive browser + extract/create.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Instant;

use obz_core::engine::{self, EngineOptions};
mod jobs;
mod desktop;
mod drag;
mod associations;
mod preview;
mod instance;
use serde::Serialize;

#[derive(Serialize)]
struct EntryView {
    name: String,
    size: u64,
    is_dir: bool,
    crc: String,
    note: String,
}

#[derive(Serialize)]
struct ArchiveView {
    kind: String,
    path: String,
    entries: Vec<EntryView>,
}

fn load_view(path: &str) -> Result<ArchiveView, String> {
    let entries = obz_core::operations::list_archive(path, &EngineOptions::default())?;
    let kind = if entries.iter().any(|e|e.method=="7z payload") {"专用安装包"} else {"归档"}.to_string();
    Ok(ArchiveView {kind,path:path.to_owned(),entries:entries.into_iter().map(|e| EntryView {
        name:e.name,size:e.size,is_dir:e.is_dir,crc:e.crc,
        note:if e.encrypted {format!("{} · 加密",e.method)} else {e.method}
    }).collect()})
}

#[tauri::command]
async fn open_archive(path: String) -> Result<ArchiveView, String> {
    tauri::async_runtime::spawn_blocking(move || load_view(&path)).await.map_err(|e|e.to_string())?
}

#[tauri::command]
async fn extract_archive(path: String, dest: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        let n = obz_core::operations::extract_archive(&path, &dest, &EngineOptions::default())?;
        Ok(format!("已解压 {n} 个文件 · {:.2}s", t0.elapsed().as_secs_f64()))
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
async fn create_zip(out: String, inputs: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = engine::create(&out, &inputs, "zip", &EngineOptions::default())?;
        Ok(format!("已创建 {}", files.join("、")))
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
fn open_tools(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(window)=app.get_webview_window("tools") {window.set_focus().map_err(|e|e.to_string())?;return Ok(());}
    tauri::WebviewWindowBuilder::new(&app,"tools",tauri::WebviewUrl::App("tools.html".into()))
        .title("qing-zip · 归档工具").inner_size(920.0,760.0).min_inner_size(780.0,600.0)
        .build().map_err(|e|e.to_string())?;
    Ok(())
}

#[tauri::command]
fn startup_path() -> Option<String> { instance::startup_paths().into_iter().next() }

#[tauri::command]
fn app_version() -> String { env!("CARGO_PKG_VERSION").to_owned() }

#[tauri::command]
fn current_executable() -> Result<String,String> { std::env::current_exe().map(|p|p.to_string_lossy().into_owned()).map_err(|e|e.to_string()) }

#[tauri::command]
fn window_maximized(window: tauri::WebviewWindow) -> Result<bool,String> {
    window.is_maximized().map_err(|e|e.to_string())
}

#[tauri::command]
fn window_action(window: tauri::WebviewWindow, action: String) -> Result<(),String> {
    match action.as_str(){
        "minimize"=>window.minimize(),
        "maximize"=>if window.is_maximized().map_err(|e|e.to_string())? {window.unmaximize()} else {window.maximize()},
        "drag"=>window.start_dragging(),
        "close"=>window.close(),
        _=>return Err("unknown window action".into())
    }.map_err(|e|e.to_string())
}

#[tauri::command]
async fn cancel_and_close(app: tauri::AppHandle) -> Result<(),String> {
    jobs::cancel_all();
    tauri::async_runtime::spawn_blocking(move || {
        for _ in 0..300 {if !jobs::running(){app.exit(0);return Ok(());}std::thread::sleep(std::time::Duration::from_millis(100));}
        Err("任务正在安全停止，请稍后再关闭窗口。".into())
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
fn set_appearance(window: tauri::WebviewWindow, material: String, dark: bool) -> Result<(),String> {
    use tauri::utils::{config::WindowEffectsConfig, WindowEffect};
    window.set_theme(Some(if dark {tauri::Theme::Dark} else {tauri::Theme::Light})).map_err(|e|e.to_string())?;
    let effects=match material.as_str(){
        "solid"=>None,
        "acrylic"=>Some(WindowEffectsConfig{effects:vec![WindowEffect::Acrylic],..Default::default()}),
        "mica"=>Some(WindowEffectsConfig{effects:vec![if dark {WindowEffect::MicaDark}else{WindowEffect::MicaLight}],..Default::default()}),
        _=>return Err("unknown material".into())
    };
    window.set_effects(effects).map_err(|e|e.to_string())
}

#[tauri::command]
fn pick_files() -> Vec<String> {
    rfd::FileDialog::new()
        .set_title("打开压缩文件")
        .add_filter("归档/安装包", &["zip", "7z", "rar", "exe", "tar", "gz", "xz", "bz2", "iso", "cab", "wim", "001", "zipx", "egg", "alz", "ace", "lzh"])
        .add_filter("所有文件", &["*"])
        .pick_files()
        .map(|v| v.iter().map(|p| p.to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

#[tauri::command]
fn pick_folder() -> String {
    rfd::FileDialog::new()
        .set_title("选择文件夹")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
fn pick_archive_save(format: String) -> String {
    let ext=match format.as_str(){"7z"=>"7z","tar"=>"tar","tgz"=>"tar.gz","tbz2"=>"tar.bz2","txz"=>"tar.xz","gzip"=>"gz","xz"=>"xz","bzip2"=>"bz2","wim"=>"wim","sfx"=>"exe","iso"=>"iso","zipx"=>"zipx","lzh"=>"lzh",_=>"zip"};
    rfd::FileDialog::new().set_title("新建压缩包").set_file_name(format!("archive.{ext}")).save_file().map(|p|p.to_string_lossy().into_owned()).unwrap_or_default()
}

#[tauri::command]
fn pick_save() -> String {
    rfd::FileDialog::new()
        .set_title("新建 zip")
        .set_file_name("archive.zip")
        .add_filter("ZIP", &["zip"])
        .save_file()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
fn pick_copy_save(path: String) -> String {
    let name=std::path::Path::new(&path).file_name().map(|n|n.to_string_lossy().into_owned()).unwrap_or_else(||"archive.zip".into());
    rfd::FileDialog::new().set_title("另存为").set_file_name(name).save_file().map(|p|p.to_string_lossy().into_owned()).unwrap_or_default()
}

#[tauri::command]
fn pick_files_for_zip() -> Vec<String> {
    rfd::FileDialog::new()
        .set_title("选择要打包的文件")
        .pick_files()
        .map(|v| v.iter().map(|p| p.to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

#[tauri::command]
async fn dropped(path: String) -> Result<ArchiveView, String> {
    open_archive(path).await
}

fn main() {
    instance::queue_startup();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(instance::receive))
        .on_window_event(|window,event|{
            if let tauri::WindowEvent::CloseRequested{api,..}=event {
                if window.label()=="main" && jobs::running(){
                    use tauri::Emitter;
                    api.prevent_close();let _=window.emit("close-requested",());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_archive,
            extract_archive,
            create_zip,
            pick_files,
            pick_folder,
            pick_save, pick_copy_save,
            pick_files_for_zip,
            dropped, open_tools, app_version, current_executable, startup_path, pick_archive_save, set_appearance, window_action, window_maximized, cancel_and_close, jobs::start_operation, jobs::operation_status, jobs::wait_operation, jobs::cancel_operation, jobs::forget_operation,
            drag::drag_paths, desktop::known_folders, desktop::reveal_path, desktop::copy_archive, desktop::clipboard_paths, desktop::clipboard_directory, desktop::recycle_paths, desktop::list_directory,
            associations::folder_menu_enabled, associations::set_folder_menu, instance::take_pending_folders, associations::association_status, associations::set_file_associations, associations::open_default_apps,
            preview::preview_entry, preview::open_archive_image, preview::clear_preview_cache, instance::take_pending_archives, instance::canonical_archive_path
        ])
        .run(tauri::generate_context!())
        .expect("qing-zip 启动失败");
}

