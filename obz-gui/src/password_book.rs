use obz_core::operations::Request;
use obz_core::services::{Vault, VaultLabel};
use std::path::Path;

pub fn track_extraction(request: &mut Request) {
    if (request.operation == "extract"
        && (request.options.password.is_some() || request.password_id.is_some()))
        || request.operation == "deep-extract"
    {
        request.options.extraction_password_use = Some(Default::default());
    }
}

fn vault() -> Result<Vault, String> {
    Vault::open(Vault::default_path()?).map_err(|_| "无法打开本机密码本".into())
}

pub fn remember<'a>(passwords: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<(), String> {
    let mut entries = passwords.into_iter().peekable();
    if entries.peek().is_none() {
        return Ok(());
    }
    vault()?
        .remember_extraction_passwords(entries.map(|(path, password)| {
            let label = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            (label, password)
        }))
        .map_err(|_| "无法保存解压密码".into())
}

#[tauri::command]
pub async fn password_book_list() -> Result<Vec<VaultLabel>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        vault()?
            .password_book_entries()
            .map_err(|_| "无法读取本机密码本".into())
    })
    .await
    .map_err(|_| "无法读取本机密码本".to_owned())?
}

#[tauri::command]
pub async fn password_book_get(id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        vault()?
            .password_book_password(&id)
            .map_err(|_| "无法读取这条密码".into())
    })
    .await
    .map_err(|_| "无法读取这条密码".to_owned())?
}

#[tauri::command]
pub async fn password_book_save(label: String, password: String) -> Result<VaultLabel, String> {
    if password.is_empty() {
        return Err("请输入密码".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        vault()?
            .save_password_book_entry(&label, &password)
            .map_err(|_| "无法保存这条密码".into())
    })
    .await
    .map_err(|_| "无法保存这条密码".to_owned())?
}

#[tauri::command]
pub async fn password_book_delete(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        vault()?
            .remove_password_book_entry(&id)
            .map(|_| ())
            .map_err(|_| "无法删除这条密码".into())
    })
    .await
    .map_err(|_| "无法删除这条密码".to_owned())?
}
