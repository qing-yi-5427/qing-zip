use obz_core::{engine::EngineOptions, installer};
fn sample() -> String {
    format!("{}/../BANDIZIP-SETUP-BETA.exe", env!("CARGO_MANIFEST_DIR"))
}
#[test]
fn installer_combined_listing_selected_ui_and_existing_payload() {
    let data = installer::recognize(&sample())
        .unwrap()
        .expect("installer fixture");
    let es = installer::list(&data).unwrap();
    assert_eq!(es.len(), 187);
    assert!(es
        .iter()
        .all(|e| e.name.starts_with("payload/") || e.name.starts_with("ui/")));
    let payload = es
        .iter()
        .find(|e| !e.is_dir && e.name.starts_with("payload/"))
        .unwrap();
    let ui = es
        .iter()
        .find(|e| !e.is_dir && e.name.starts_with("ui/"))
        .unwrap();
    let dest = tempfile::tempdir().unwrap();
    let existing = dest.path().join(&payload.name);
    std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
    std::fs::write(&existing, b"preserve existing").unwrap();
    assert!(installer::extract(
        &data,
        dest.path().to_str().unwrap(),
        &EngineOptions::default()
    )
    .is_err());
    assert_eq!(std::fs::read(&existing).unwrap(), b"preserve existing");
    let mut opts = EngineOptions::default();
    opts.selected = vec![ui.name.clone()];
    assert_eq!(
        installer::extract(&data, dest.path().to_str().unwrap(), &opts).unwrap(),
        1
    );
    let preview = installer::preview(&data, &ui.name, &opts).unwrap();
    assert_eq!(preview, std::fs::read(dest.path().join(&ui.name)).unwrap());
    assert_eq!(preview.len() as u64, ui.size);
    assert_eq!(std::fs::read(&existing).unwrap(), b"preserve existing");
    opts.overwrite = "skip".into();
    assert_eq!(
        installer::extract(&data, dest.path().to_str().unwrap(), &opts).unwrap(),
        0
    );
    installer::test(&data, &EngineOptions::default()).unwrap();
}
#[test]
fn arbitrary_mz_is_not_an_installer_and_precancel_is_obeyed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fake.exe");
    for size in [2, 64, 128, 512, 4096] {
        let mut b = vec![0xff; size];
        b[..2].copy_from_slice(b"MZ");
        std::fs::write(&file, b).unwrap();
        assert!(installer::recognize(file.to_str().unwrap())
            .unwrap()
            .is_none());
    }
    let data = installer::recognize(&sample()).unwrap().unwrap();
    let mut opts = EngineOptions::default();
    let ctl = obz_core::engine::JobControl::new();
    ctl.cancel();
    opts.control = Some(ctl);
    assert!(installer::test(&data, &opts).is_err());
}
#[test]
fn corrupt_selected_ui_does_not_publish_any_file() {
    let mut data = installer::recognize(&sample()).unwrap().unwrap();
    let pkg = obz_core::setup::open(&data).unwrap();
    let ui = pkg
        .ui_entries
        .iter()
        .find(|e| e.comp_size > 4 && !e.name.ends_with('/'))
        .unwrap();
    let name = format!("ui/{}", ui.name);
    let local = pkg.zip_offset + ui.local_offset as usize;
    let name_len = u16::from_le_bytes([data[local + 26], data[local + 27]]) as usize;
    let extra_len = u16::from_le_bytes([data[local + 28], data[local + 29]]) as usize;
    let at = local + 30 + name_len + extra_len;
    data[at] ^= 0xff;
    let out = tempfile::tempdir().unwrap();
    let mut opts = EngineOptions::default();
    opts.selected = vec![name];
    assert!(installer::extract(&data, out.path().to_str().unwrap(), &opts).is_err());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}
