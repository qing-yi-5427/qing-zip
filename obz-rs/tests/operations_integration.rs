//! Exercise the shipped CLI JSON dispatcher, not only individual core methods.
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, process::{Command, Stdio}};

fn call(root: &Path, request: Value) -> Result<Value, String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_obz"))
        .arg("json").env("LOCALAPPDATA", root.join("profile"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(serde_json::to_string(&request).unwrap().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
    serde_json::from_slice(&output.stdout).map_err(|e|e.to_string())
}

#[test]
fn cli_recovery_finds_only_valid_encrypted_passwords() {
    let t = tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("secret.txt"); fs::write(&input,b"encrypted fixture").unwrap();
    let archive=p.join("secret.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[input],"options":{"password":"ba"}})).unwrap();
    let found=call(p,json!({"operation":"recover","path":archive,"candidates":["wrong","ba"],"max_attempts":3})).unwrap();
    assert_eq!(found["status"],"found"); assert_eq!(found["password"],"ba"); assert_eq!(found["attempts"],2);
    let exhausted=call(p,json!({"operation":"recover","path":archive,"candidates":["wrong"],"max_attempts":2})).unwrap();
    assert_eq!(exhausted["status"],"exhausted"); assert!(exhausted["password"].is_null());
    let charset=call(p,json!({"operation":"recover","path":archive,"charset":"ab","min_length":2,"max_length":2,"max_attempts":4})).unwrap();
    assert_eq!(charset["password"],"ba"); assert_eq!(charset["attempts"],3);
    let plain=p.join("plain.zip"); call(p,json!({"operation":"create","path":plain,"inputs":[input]})).unwrap();
    assert!(call(p,json!({"operation":"recover","path":plain,"candidates":["anything"]})).unwrap_err().contains("no encrypted"));
}

#[test]
fn cli_smart_and_selected_extraction_are_real() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let a=p.join("a.txt"); let b=p.join("b.txt");
    fs::write(&a,b"a").unwrap(); fs::write(&b,b"b").unwrap(); let archive=p.join("bundle.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[a,b]})).unwrap();
    let dest=p.join("out"); let result=call(p,json!({"operation":"extract","path":archive,"dest":dest,"smart":true})).unwrap();
    assert_eq!(result["files"],2); assert_eq!(fs::read(dest.join("bundle/a.txt")).unwrap(),b"a");
    let selected=p.join("selected"); call(p,json!({"operation":"extract","path":archive,"dest":selected,"options":{"selected":["b.txt"]}})).unwrap();
    assert_eq!(fs::read(selected.join("b.txt")).unwrap(),b"b"); assert!(!selected.join("a.txt").exists());
}

#[test]
fn cli_preview_returns_images_but_not_active_content() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let image=p.join("pixel.gif"); let active=p.join("active.svg");
    fs::write(&image,b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00\x3b").unwrap();
    fs::write(&active,b"<svg onload='alert(1)'/>").unwrap(); let archive=p.join("images.zip");
    call(p,json!({"operation":"create","path":archive,"inputs":[image,active]})).unwrap();
    let result=call(p,json!({"operation":"preview","path":archive,"name":"pixel.gif"})).unwrap();
    assert_eq!(result["mime"],"image/gif"); assert!(result["data_url"].as_str().unwrap().starts_with("data:image/gif;base64,R0lG"));
    assert!(call(p,json!({"operation":"preview","path":archive,"name":"active.svg"})).is_err());
}

#[cfg(windows)]
#[test]
fn cli_vault_isolated_profile_integrates_password_id() {
    let t=tempfile::tempdir().unwrap(); let p=t.path();
    call(p,json!({"operation":"vault-put","name":"fixture","label":"Test","options":{"password":"vault-fixture-secret"}})).unwrap();
    let list=call(p,json!({"operation":"vault-list"})).unwrap(); assert_eq!(list[0]["id"],"fixture"); assert!(!list.to_string().contains("vault-fixture-secret"));
    let bytes=fs::read(p.join("profile/OpenBandiZip/passwords.dpapi")).unwrap(); assert!(!bytes.windows(20).any(|w|w==b"vault-fixture-secret"));
    let source=p.join("file.txt"); fs::write(&source,b"secret content").unwrap(); let archive=p.join("vault.7z");
    call(p,json!({"operation":"create","path":archive,"inputs":[source],"password_id":"fixture"})).unwrap();
    assert_eq!(call(p,json!({"operation":"test","path":archive,"password_id":"fixture"})).unwrap()["valid"],true);
    assert!(call(p,json!({"operation":"test","path":archive,"options":{"password":"wrong"}})).is_err());
    call(p,json!({"operation":"vault-delete","name":"fixture"})).unwrap(); assert_eq!(call(p,json!({"operation":"vault-list"})).unwrap(),json!([]));
}

#[cfg(windows)]
#[test]
fn cli_iso_creation_dispatch_roundtrips() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("中文.txt"); fs::write(&input,b"ISO payload").unwrap(); let archive=p.join("image.iso");
    let result=call(p,json!({"operation":"create","path":archive,"inputs":[input],"format":"iso"})).unwrap(); assert!(result["outputs"][0].is_string());
    let result=call(p,json!({"operation":"list","path":archive})).unwrap(); assert!(result["entries"].as_array().unwrap().iter().any(|e|e["name"]=="中文.txt"));
    let dest=p.join("out"); call(p,json!({"operation":"extract","path":archive,"dest":dest})).unwrap(); assert_eq!(fs::read(dest.join("中文.txt")).unwrap(),b"ISO payload");
}

#[test]
fn cli_repair_dispatch_preserves_original() {
    let t=tempfile::tempdir().unwrap(); let p=t.path(); let input=p.join("data.txt"); fs::write(&input,b"recover me").unwrap(); let archive=p.join("broken.zip");
    obz_core::zipfmt::create_zip(archive.to_str().unwrap(),&[input.to_string_lossy().into_owned()],false).unwrap();
    let mut broken=fs::read(&archive).unwrap(); let cd=broken.windows(4).position(|w|w==b"PK\x01\x02").unwrap(); broken.truncate(cd); fs::write(&archive,&broken).unwrap();
    let fixed=p.join("fixed.zip"); let report=call(p,json!({"operation":"repair","path":archive,"dest":fixed})).unwrap();
    assert_eq!(report["recovered"],json!(["data.txt"])); assert_eq!(report["complete"],false); assert_eq!(fs::read(archive).unwrap(),broken);
    assert_eq!(call(p,json!({"operation":"test","path":fixed})).unwrap()["valid"],true);
}
