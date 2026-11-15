//! Pure-byte parts, independent of archive volumes. All output is no-clobber.
use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
const MANIFEST: &str = "qing-zip-parts.json";
const MAX_PARTS: usize = 10000;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    name: String,
    size: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    name: String,
    size: u64,
    sha256: String,
    parts: Vec<Part>,
}
fn leaf(name: &str) -> Result<(), String> {
    let relative = validate_name(name)?;
    if relative.components().count() != 1 {
        return Err("文件名称必须是单个文件名".into());
    }
    Ok(())
}
fn copy_hash(
    input: &mut impl Read,
    output: &mut impl Write,
    size: u64,
    total: u64,
    done: &mut u64,
    whole: &mut Sha256,
    ctl: &JobControl,
) -> Result<String, String> {
    let mut remaining = size;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    while remaining != 0 {
        ctl.check()?;
        let amount = remaining.min(buffer.len() as u64) as usize;
        input
            .read_exact(&mut buffer[..amount])
            .map_err(|e| e.to_string())?;
        output
            .write_all(&buffer[..amount])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..amount]);
        whole.update(&buffer[..amount]);
        remaining -= amount as u64;
        *done += amount as u64;
        if ctl.snapshot().phase != "publishing" {
            ctl.set(
                "copying",
                Some((u128::from(*done) * 95 / u128::from(total.max(1))) as u8),
                "正在处理文件",
            );
        }
    }
    ctl.check()?;
    Ok(hex(&hash.finalize()))
}
fn parent_guard(parent: &Path) -> Result<CheckedDirectories, String> {
    if !parent.is_dir() {
        return Err("请选择已存在的目标文件夹".into());
    }
    let mut guards = CheckedDirectories::default();
    guards.check(parent)?;
    Ok(guards)
}
pub fn split(
    path: &str,
    destination: &str,
    block: u64,
    opts: &EngineOptions,
) -> Result<Value, String> {
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    if block == 0 {
        return Err("分块大小必须大于零".into());
    }
    let source = absolute(path)?;
    let parent = absolute(destination)?;
    let mut input = crate::engine_stream::pin(&source)?;
    let size = input.metadata().map_err(|e| e.to_string())?.len();
    let count = size.div_ceil(block).max(1);
    if count > MAX_PARTS as u64 {
        return Err("分块数量超过10000，请增大分块大小".into());
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("无效源文件名")?
        .to_owned();
    leaf(&name)?;
    let dest = parent.join(format!("{name}.parts"));
    let mut guards = parent_guard(&parent)?;
    guards.check(&dest)?;
    if dest.try_exists().map_err(|e| e.to_string())? {
        return Err("目标分块文件夹已存在".into());
    }
    let stage = new_fallback_stage(&parent, &mut guards)?;
    let mut stage_guards = stage.guards(None)?;
    let zone = crate::motw::archive_context(&source)?;
    let mut whole = Sha256::new();
    let mut done = 0;
    let mut parts = Vec::new();
    let digits = 3;
    for index in 1..=count {
        ctl.check()?;
        let part_name = format!("{name}.{index:0digits$}");
        leaf(&part_name)?;
        let amount = (size - done).min(block);
        let mut file = create_staged_output(&stage.path().join(&part_name), &mut stage_guards)?;
        let hash = copy_hash(
            &mut *input,
            &mut file,
            amount,
            size,
            &mut done,
            &mut whole,
            &ctl,
        )?;
        file.sync_all().map_err(|e| e.to_string())?;
        parts.push(Part {
            name: part_name,
            size: amount,
            sha256: hash,
        });
    }
    let manifest = Manifest {
        version: 1,
        name,
        size,
        sha256: hex(&whole.finalize()),
        parts,
    };
    let mut file = create_staged_output(&stage.path().join(MANIFEST), &mut stage_guards)?;
    serde_json::to_writer(&mut file, &manifest).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    ctl.check()?;
    guards.allow_publication(&parent)?;
    let parent_handle = File::options()
        .access_mode(0x200a0)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(&parent)
        .map_err(|e| e.to_string())?;
    validate_transaction_root(&parent_handle, &parent)?;
    let created =
        crate::private_stage::create_journal_directory(&parent_handle, dest.file_name().unwrap())?;
    guards.exclusive_claim = true;
    guards.created_output_dirs.push((
        dest.clone(),
        created_directories::Identity::owned(&created)?,
    ));
    guards.selected_output_root = Some(dest.clone());
    guards.owned_dirs.insert(dest.clone());
    drop(created);
    let journal = PublicationJournal::default();
    ctl.set("publishing", Some(95), "正在保存分块文件");
    let result = (|| {
        guards.check(&dest)?;
        guards.allow_publication(&dest)?;
        for part_name in manifest
            .parts
            .iter()
            .map(|p| p.name.as_str())
            .chain(std::iter::once(MANIFEST))
        {
            ctl.check()?;
            let target = dest.join(part_name);
            guards.check(&target)?;
            with_output_temp(&dest, &guards, |out| {
                let mut input =
                    crate::engine_stream::open_source_leaf(&stage.path().join(part_name))
                        .map_err(|e| e.to_string())?;
                let length = input.metadata().map_err(|e| e.to_string())?.len();
                let mut copied = 0;
                copy_hash(
                    &mut input,
                    out.as_file_mut(),
                    length,
                    length,
                    &mut copied,
                    &mut Sha256::new(),
                    &ctl,
                )?;
                crate::motw::apply_to_file(out.as_file(), Some(&zone))?;
                out.as_file().sync_all().map_err(|e| e.to_string())?;
                ctl.check()?;
                publish_output_recorded(out, &target, false, Some(&journal)).map(drop)
            })?;
        }
        Ok::<(), String>(())
    })();
    if let Err(reason) = result {
        let errors = journal.rollback();
        drop(guards);
        if !errors.is_empty() || dest.try_exists().unwrap_or(true) {
            return Err(format!(
                "{reason}; 部分目标保留（外来或无法清理的文件）：{} {}",
                dest.display(),
                errors.join("; ")
            ));
        }
        return Err(reason);
    }
    guards.created_output_dirs.clear();
    ctl.set("complete", Some(100), "文件分割完成");
    Ok(json!({"output":dest,"files":count,"bytes":size,"sha256":manifest.sha256}))
}
fn sequence(first: &Path) -> Result<(String, usize), String> {
    let name = first
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("无效分块名称")?;
    let (base, number) = name.rsplit_once('.').ok_or("请选择 .001 首卷")?;
    if number.len() < 3
        || !number.bytes().all(|c| c.is_ascii_digit())
        || number.parse::<usize>().ok() != Some(1)
    {
        return Err("请选择编号为001的首卷，不能从中间卷合并".into());
    }
    leaf(base)?;
    Ok((base.to_owned(), number.len()))
}
pub fn combine(
    path: &str,
    destination: &str,
    expected_size: Option<u64>,
    opts: &EngineOptions,
) -> Result<Value, String> {
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    let first = absolute(path)?;
    let dest = absolute(destination)?;
    let (base, width) = sequence(&first)?;
    let directory = first.parent().ok_or("无效分块目录")?;
    let parent = dest.parent().ok_or("无效目标文件")?;
    let mut guards = parent_guard(parent)?;
    guards.check(&dest)?;
    if dest.try_exists().map_err(|e| e.to_string())? {
        return Err("目标文件已存在，不能覆盖".into());
    }
    // Pin the first source and ancestors before enumerating any sibling names.
    let _first_pin = crate::engine_stream::pin(&first)?;
    let manifest_path = directory.join(MANIFEST);
    let mut manifest_pin = None;
    let has_manifest = match fs::symlink_metadata(&manifest_path) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    let manifest = if has_manifest {
        let mut file = crate::engine_stream::pin(&manifest_path)?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("分块清单过大".into());
        }
        let value: Manifest =
            serde_json::from_reader(&mut *file).map_err(|e| format!("分块清单无效：{e}"))?;
        if value.version != 1
            || value.name != base
            || value.parts.is_empty()
            || value.parts.len() > MAX_PARTS
        {
            return Err("分块清单与首卷不匹配".into());
        }
        manifest_pin = Some(file);
        Some(value)
    } else {
        None
    };
    let mut discovered = BTreeMap::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        ctl.check()?;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(suffix) = name.strip_prefix(&format!("{base}.")) else {
            continue;
        };
        if !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit()) {
            let number = suffix.parse::<usize>().map_err(|_| "分块编号过大")?;
            if suffix != format!("{number:0width$}")
                || number == 0
                || number > MAX_PARTS
                || discovered.insert(number, entry.path()).is_some()
            {
                return Err("分块编号重复或格式错误".into());
            }
        }
    }
    if discovered.is_empty() {
        return Err("未找到分块文件".into());
    }
    for (index, number) in discovered.keys().enumerate() {
        if *number != index + 1 {
            return Err(format!("缺少第{}卷，无法合并", index + 1));
        }
    }
    if manifest
        .as_ref()
        .is_some_and(|m| m.parts.len() != discovered.len())
    {
        return Err("分块数量与清单不一致，可能缺少末卷或存在多余卷".into());
    }
    let mut inputs = Vec::new();
    let mut total = 0u64;
    let mut zone = if has_manifest {
        crate::motw::archive_context(&manifest_path)?
    } else {
        crate::motw::ZoneIdentifier::unmarked()
    };
    for (index, path) in discovered.values().enumerate() {
        ctl.check()?;
        let input = crate::engine_stream::pin(path)?;
        let part_zone = crate::motw::archive_context(path)?;
        if part_zone.more_restrictive_than(&zone) {
            zone = part_zone;
        }
        let size = input.metadata().map_err(|e| e.to_string())?.len();
        if let Some(m) = &manifest {
            let p = &m.parts[index];
            leaf(&p.name)?;
            if path.file_name().and_then(|n| n.to_str()) != Some(&p.name) || p.size != size {
                return Err("分块名称、顺序或长度与清单不一致".into());
            }
        }
        total = total.checked_add(size).ok_or("合并文件长度溢出")?;
        inputs.push((input, size));
    }
    if expected_size.is_some_and(|n| n != total)
        || manifest.as_ref().is_some_and(|m| m.size != total)
    {
        return Err("总长度不匹配，可能缺卷或分块已更改".into());
    }
    guards.allow_publication(parent)?;
    let digest = with_output_temp(parent, &guards, |output| {
        let mut whole = Sha256::new();
        let mut done = 0;
        for (index, (input, size)) in inputs.iter_mut().enumerate() {
            let hash = copy_hash(
                &mut **input,
                output.as_file_mut(),
                *size,
                total,
                &mut done,
                &mut whole,
                &ctl,
            )?;
            if manifest
                .as_ref()
                .is_some_and(|m| m.parts[index].sha256 != hash)
            {
                return Err(format!("第{}卷校验失败", index + 1));
            }
        }
        let hash = hex(&whole.finalize());
        if manifest.as_ref().is_some_and(|m| m.sha256 != hash) {
            return Err("合并文件SHA256与清单不一致".into());
        }
        crate::motw::apply_to_file(output.as_file(), Some(&zone))?;
        output.as_file().sync_all().map_err(|e| e.to_string())?;
        ctl.check()?;
        publish_output(output, &dest, false)?;
        Ok(hash)
    })?;
    drop(manifest_pin);
    ctl.set("complete", Some(100), "文件合并完成");
    Ok(
        json!({"output":dest,"files":inputs.len(),"bytes":total,"sha256":digest,"verified_manifest":manifest.is_some()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(size: usize) -> (tempfile::TempDir, PathBuf, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("随机文件.bin");
        let bytes = (0..size)
            .map(|i| ((i * 73 + i / 251) % 256) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        (dir, path, bytes)
    }
    #[test]
    fn roundtrip_empty_unicode_multivolume_and_unmanifested() {
        for size in [0, 1, 8199] {
            let (dir, path, bytes) = fixture(size);
            let r = split(
                path.to_str().unwrap(),
                dir.path().to_str().unwrap(),
                1000,
                &EngineOptions::default(),
            )
            .unwrap();
            let parts = PathBuf::from(r["output"].as_str().unwrap());
            let first = parts.join("随机文件.bin.001");
            let dest = dir.path().join("合并.bin");
            let result = combine(
                first.to_str().unwrap(),
                dest.to_str().unwrap(),
                None,
                &EngineOptions::default(),
            )
            .unwrap();
            assert_eq!(result["verified_manifest"], true);
            assert_eq!(fs::read(&dest).unwrap(), bytes);
            fs::remove_file(parts.join(MANIFEST)).unwrap();
            let raw = dir.path().join("raw.bin");
            assert_eq!(
                combine(
                    first.to_str().unwrap(),
                    raw.to_str().unwrap(),
                    None,
                    &EngineOptions::default()
                )
                .unwrap()["verified_manifest"],
                false
            );
            assert_eq!(fs::read(raw).unwrap(), bytes);
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn missing_changed_wrong_order_and_existing_outputs_are_rejected() {
        let (dir, path, bytes) = fixture(3500);
        let opts = EngineOptions::default();
        split(
            path.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            1000,
            &opts,
        )
        .unwrap();
        let parts = dir.path().join("随机文件.bin.parts");
        let first = parts.join("随机文件.bin.001");
        let dest = dir.path().join("out");
        assert!(combine(
            parts.join("随机文件.bin.002").to_str().unwrap(),
            dest.to_str().unwrap(),
            None,
            &opts
        )
        .is_err());
        let last = parts.join("随机文件.bin.004");
        let last_bytes = fs::read(&last).unwrap();
        fs::remove_file(&last).unwrap();
        assert!(combine(first.to_str().unwrap(), dest.to_str().unwrap(), None, &opts).is_err());
        fs::write(last, last_bytes).unwrap();
        let second = parts.join("随机文件.bin.002");
        fs::write(&second, vec![0; 1000]).unwrap();
        assert!(
            combine(first.to_str().unwrap(), dest.to_str().unwrap(), None, &opts)
                .unwrap_err()
                .contains("校验失败")
        );
        assert!(!dest.exists());
        fs::write(&dest, b"existing").unwrap();
        assert!(combine(first.to_str().unwrap(), dest.to_str().unwrap(), None, &opts).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"existing");
        assert!(split(
            path.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            1000,
            &opts
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        fs::remove_file(parts.join(MANIFEST)).unwrap();
        fs::remove_file(second).unwrap();
        fs::remove_file(dest.clone()).unwrap();
        assert!(
            combine(first.to_str().unwrap(), dest.to_str().unwrap(), None, &opts)
                .unwrap_err()
                .contains("缺少")
        );
    }
    #[test]
    fn publication_cancel_rolls_back_and_conflict_preserves_foreign_file() {
        for conflict in [false, true] {
            let (dir, path, bytes) = fixture(16 * 1024 * 1024);
            let ctl = JobControl::new();
            let opts = EngineOptions {
                control: Some(ctl.clone()),
                ..Default::default()
            };
            let dest = dir.path().join("随机文件.bin.parts");
            let last = dest.join("随机文件.bin.064");
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| {
                    split(
                        path.to_str().unwrap(),
                        dir.path().to_str().unwrap(),
                        256 * 1024,
                        &opts,
                    )
                });
                let mut triggered = false;
                while !worker.is_finished() {
                    if ctl.snapshot().phase == "publishing" {
                        if conflict {
                            fs::write(&last, b"foreign").unwrap();
                        } else {
                            ctl.cancel();
                        }
                        triggered = true;
                        break;
                    }
                    std::thread::yield_now();
                }
                assert!(triggered);
                assert!(worker.join().unwrap().is_err());
            });
            if conflict {
                assert_eq!(fs::read(last).unwrap(), b"foreign");
                assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
            } else {
                assert!(!dest.exists());
            }
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn later_volume_internet_zone_is_preserved_and_optional_length_is_checked() {
        let (dir, path, _) = fixture(2500);
        let opts = EngineOptions::default();
        split(
            path.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            1000,
            &opts,
        )
        .unwrap();
        let parts = dir.path().join("随机文件.bin.parts");
        let first = parts.join("随机文件.bin.001");
        fs::write(
            format!("{}:Zone.Identifier", first.display()),
            b"[ZoneTransfer]\r\nZoneId=1\r\n",
        )
        .unwrap();
        fs::write(
            format!(
                "{}:Zone.Identifier",
                parts.join("随机文件.bin.002").display()
            ),
            b"[ZoneTransfer]\r\nZoneId=4\r\n",
        )
        .unwrap();
        let dest = dir.path().join("merged");
        assert!(combine(
            first.to_str().unwrap(),
            dest.to_str().unwrap(),
            Some(2499),
            &opts
        )
        .is_err());
        assert!(!dest.exists());
        combine(
            first.to_str().unwrap(),
            dest.to_str().unwrap(),
            Some(2500),
            &opts,
        )
        .unwrap();
        assert!(
            fs::read_to_string(format!("{}:Zone.Identifier", dest.display()))
                .unwrap()
                .contains("ZoneId=4")
        );
    }
    #[test]
    fn numbering_crosses_999_and_accepts_fixed_width_third_party_parts() {
        let (dir, path, bytes) = fixture(1001);
        let opts = EngineOptions::default();
        split(
            path.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            1,
            &opts,
        )
        .unwrap();
        let parts = dir.path().join("随机文件.bin.parts");
        assert!(parts.join("随机文件.bin.999").exists());
        assert!(parts.join("随机文件.bin.1000").exists());
        let first = parts.join("随机文件.bin.001");
        let merged = dir.path().join("all");
        combine(
            first.to_str().unwrap(),
            merged.to_str().unwrap(),
            None,
            &opts,
        )
        .unwrap();
        assert_eq!(fs::read(merged).unwrap(), bytes);
        fs::remove_file(parts.join(MANIFEST)).unwrap();
        for i in 1..1000 {
            fs::rename(
                parts.join(format!("随机文件.bin.{i:03}")),
                parts.join(format!("随机文件.bin.{i:04}")),
            )
            .unwrap();
        }
        let merged = dir.path().join("fixed");
        combine(
            parts.join("随机文件.bin.0001").to_str().unwrap(),
            merged.to_str().unwrap(),
            None,
            &opts,
        )
        .unwrap();
        assert_eq!(fs::read(merged).unwrap(), bytes);
    }
    #[test]
    fn cancelled_and_linked_sources_do_not_publish() {
        let (dir, path, bytes) = fixture(10);
        let ctl = JobControl::new();
        ctl.cancel();
        let opts = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(split(
            path.to_str().unwrap(),
            dir.path().to_str().unwrap(),
            2,
            &opts
        )
        .is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        let linked = dir.path().join("link.bin");
        if std::os::windows::fs::symlink_file(&path, &linked).is_ok() {
            assert!(split(
                linked.to_str().unwrap(),
                dir.path().to_str().unwrap(),
                2,
                &EngineOptions::default()
            )
            .is_err());
        }
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
