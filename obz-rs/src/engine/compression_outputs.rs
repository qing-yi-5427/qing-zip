use super::*;

/// Suggest unused output paths without creating directories or reserving files.
/// Actual publication still checks every output and never overwrites it.
pub fn plan_compression_outputs(paths: &[String], format: &str, volume_bytes: Option<u64>) -> Result<Vec<String>, String> {
    if volume_bytes == Some(0) { return Err("分卷大小必须大于零".into()); }
    let mut directories: HashMap<String, (HashSet<String>, HashSet<String>)> = HashMap::new();
    let mut results = Vec::with_capacity(paths.len());
    for requested in paths {
        let path = std::path::absolute(requested).map_err(|e| e.to_string())?;
        let parent = path.parent().ok_or("输出路径缺少目录")?;
        no_links(parent)?;
        let filename = path.file_name().and_then(|name| name.to_str()).ok_or("输出文件名无效")?;
        validate_name(filename)?;
        let directory_key = path_key(parent);
        if !directories.contains_key(&directory_key) {
            let mut exact = HashSet::new();
            let mut volume_bases = HashSet::new();
            match fs::read_dir(parent) {
                Ok(entries) => for entry in entries {
                    let name = entry.map_err(|e| e.to_string())?.file_name().to_string_lossy().to_lowercase();
                    if let Some(base) = volume_base(&name) { volume_bases.insert(base.to_owned()); }
                    exact.insert(name);
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.to_string()),
            }
            directories.insert(directory_key.clone(), (exact, volume_bases));
        }
        let (exact, volume_bases) = directories.get_mut(&directory_key).unwrap();
        let compound = match format {
            "tgz" | "tar.gz" => Some(".tar.gz"),
            "tbz2" | "tar.bz2" => Some(".tar.bz2"),
            "txz" | "tar.xz" => Some(".tar.xz"),
            "tar.zst" => Some(".tar.zst"),
            _ => None,
        };
        let split = compound.filter(|suffix| filename.to_ascii_lowercase().ends_with(suffix))
            .map(|suffix| filename.len() - suffix.len())
            .or_else(|| filename.rfind('.').filter(|&index| index > 0))
            .unwrap_or(filename.len());
        let (stem, suffix) = filename.split_at(split);
        let mut chosen = None;
        for index in 1..100_002 {
            let candidate = if index == 1 { filename.to_owned() } else { format!("{stem} ({index}){suffix}") };
            let key = candidate.to_lowercase();
            if exact.contains(&key) || volume_bases.contains(&key) { continue; }
            if let Some(base) = volume_base(&key) { volume_bases.insert(base.to_owned()); }
            exact.insert(key);
            chosen = Some(if index == 1 { requested.clone() } else { parent.join(candidate).display().to_string() });
            break;
        }
        results.push(chosen.ok_or("无法找到可用的输出文件名")?);
    }
    Ok(results)
}

fn volume_base(name: &str) -> Option<&str> {
    let (base, suffix) = name.rsplit_once('.')?;
    (suffix.len() >= 3 && suffix.bytes().all(|byte| byte.is_ascii_digit())).then_some(base)
}

pub(super) fn require_free_volume_base(output: &Path, ctl: &JobControl) -> Result<(), String> {
    no_links(output)?;
    let parent = output.parent().ok_or("output parent missing")?;
    let base = output.file_name().ok_or("output name missing")?.to_string_lossy().to_lowercase();
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        ctl.check()?;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == base || volume_base(&name) == Some(base.as_str()) {
            return Err(format!("output exists: {}", entry.path().display()));
        }
    }
    Ok(())
}

pub(super) fn publish(parts: &[(PathBuf, PathBuf)], ctl: &JobControl, preserve_time: bool) -> Result<Vec<String>, String> {
    if let [(source, target)] = parts {
        publish_new_with_time(source, target, ctl, preserve_time)?;
        return Ok(vec![target.display().to_string()]);
    }
    for (_, target) in parts {
        ctl.check()?;
        no_links(target)?;
        if target.try_exists().map_err(|e| e.to_string())? {
            return Err(format!("output exists: {}", target.display()));
        }
    }
    #[cfg(windows)]
    let journal = PublicationJournal::default();
    let mut outputs = Vec::with_capacity(parts.len());
    ctl.set("publishing", None, "正在保存分卷压缩包");
    let result: Result<(), String> = (|| {
        #[cfg(windows)]
        let mut guards = CheckedDirectories::default();
        for (source, target) in parts {
            ctl.check()?;
            #[cfg(windows)]
            {
                // The guard uses Windows-native path spelling to open each
                // ancestor. Normalize ordinary paths accepting either slash.
                let native_target = std::path::absolute(target).map_err(|e| e.to_string())?;
                let parent = native_target.parent().ok_or("output parent missing")?;
                guards.check(&native_target)?;
                guards.allow_publication(parent)?;
                with_output_temp(parent, &guards, |out| {
                    let mut input = crate::engine_stream::pin(source)?;
                    let mut buffer = [0; 256 * 1024];
                    loop {
                        ctl.check()?;
                        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
                        if count == 0 { break; }
                        std::io::Write::write_all(out.as_file_mut(), &buffer[..count]).map_err(|e| e.to_string())?;
                    }
                    if preserve_time {
                        let modified = input.metadata().and_then(|meta| meta.modified()).map_err(|e| e.to_string())?;
                        out.as_file().set_times(fs::FileTimes::new().set_modified(modified)).map_err(|e| e.to_string())?;
                    }
                    out.as_file().sync_all().map_err(|e| e.to_string())?;
                    ctl.check()?;
                    publish_output_recorded(out, &native_target, false, Some(&journal)).map(drop)
                })?;
            }
            #[cfg(not(windows))]
            publish_new_with_time(source, target, ctl, preserve_time)?;
            outputs.push(target.display().to_string());
        }
        ctl.check()
    })();
    if let Err(reason) = result {
        #[cfg(windows)]
        {
            let errors = journal.rollback();
            if !errors.is_empty() {
                return Err(format!("{reason}\n部分分卷未能撤回：{}", errors.join("\n")));
            }
        }
        #[cfg(not(windows))]
        if !outputs.is_empty() {
            return Err(format!("{reason}\n已创建的分卷：{}", outputs.join("\n")));
        }
        return Err(reason);
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_skips_existing_base_later_volumes_and_previous_batch_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Output.zip");
        fs::write(dir.path().join("output.zip.003"), b"previous split").unwrap();
        fs::write(dir.path().join("Output (2).zip"), b"previous archive").unwrap();
        let paths = vec![path.display().to_string(); 2];
        let chosen = plan_compression_outputs(&paths, "zip", Some(1024)).unwrap();
        assert_eq!(chosen, ["Output (3).zip", "Output (4).zip"].map(|name| dir.path().join(name).display().to_string()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn ordinary_split_never_publishes_first_part_when_later_target_exists() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.txt");
        fs::write(&input, vec![7; 1024]).unwrap();
        let out = dir.path().join("out.zip");
        let collision = dir.path().join("out.zip.002");
        fs::write(&collision, b"existing part").unwrap();
        let opts = EngineOptions { volume_bytes: Some(128), method: Some("store".into()), ..Default::default() };
        assert!(create(out.to_str().unwrap(), &[input.display().to_string()], "zip", &opts).is_err());
        assert!(!dir.path().join("out.zip.001").exists());
        assert_eq!(fs::read(collision).unwrap(), b"existing part");
        assert_eq!(fs::read(input).unwrap(), vec![7; 1024]);
    }
}
