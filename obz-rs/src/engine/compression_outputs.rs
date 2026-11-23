use super::*;
use std::ffi::OsString;

/// Suggest unused output paths without creating directories or reserving files.
/// Actual publication still checks every output and never overwrites it.
/// `sources[i]` lists the inputs of `paths[i]`; source problems that would only
/// surface after earlier outputs of a batch were written are reported here.
pub fn plan_compression_outputs(paths: &[String], format: &str, volume_bytes: Option<u64>, sources: &[Vec<String>]) -> Result<Vec<String>, String> {
    if volume_bytes == Some(0) { return Err("分卷大小必须大于零".into()); }
    if sources.len() != paths.len() { return Err("来源与输出数量不一致".into()); }
    let mut directories: HashMap<String, (HashSet<String>, HashSet<String>)> = HashMap::new();
    let mut output_parents: HashMap<OsString, PathBuf> = HashMap::new();
    let mut results = Vec::with_capacity(paths.len());
    for (requested, inputs) in paths.iter().zip(sources) {
        let path = std::path::absolute(requested).map_err(|e| e.to_string())?;
        let parent = path.parent().ok_or("输出路径缺少目录")?;
        no_links(parent)?;
        // Exact path spelling keeps distinct parents separate on case-sensitive volumes.
        let cached = output_parents.get(parent.as_os_str());
        let resolved = if cached.is_none() {
            canonical_output_parent(parent)
        } else {
            None
        };
        let output_dir = cached.or(resolved.as_ref()).map(PathBuf::as_path);
        check_sources(output_dir, &output_parents, inputs, format)?;
        if let Some(resolved) = resolved {
            output_parents.insert(parent.as_os_str().to_owned(), resolved);
        }
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

/// Report the create-time source errors before any output of a batch exists.
/// Missing sources are left to creation, which names them.
fn check_sources(
    output_dir: Option<&Path>,
    earlier_output_dirs: &HashMap<OsString, PathBuf>,
    inputs: &[String],
    format: &str,
) -> Result<(), String> {
    let mut roots = Vec::with_capacity(inputs.len());
    for input in inputs {
        roots.push(std::path::absolute(input).map_err(|e| e.to_string())?);
    }
    let single_stream = matches!(format, "gzip" | "bzip2" | "xz") || crate::stream_formats::canonical_raw_format(format).is_some();
    if single_stream {
        if let Some(folder) = roots.iter().find(|root| root.is_dir()) {
            return Err(format!("{SINGLE_STREAM_FOLDER}\n{}", folder.display()));
        }
        if roots.len() > 1 { return Err("所选格式每个压缩包只接受一个文件。请调整来源或更改格式。".into()); }
        return Ok(());
    }
    // ISO stages its input outside the source tree. LZH lists its input before
    // it creates a candidate. Both can safely publish inside their own source.
    let own_output_inside_source_is_safe = matches!(format, "iso" | "lzh");
    for root in &roots {
        if !root.is_dir() { continue; }
        let Ok(source) = fs::canonicalize(root) else { continue };
        if earlier_output_dirs.values().any(|output| output.starts_with(&source)) {
            return Err(format!("{EARLIER_OUTPUT_IN_SOURCE}\n{}", root.display()));
        }
        if !own_output_inside_source_is_safe && output_dir.is_some_and(|output| output.starts_with(&source)) {
            return Err(format!("{OUTPUT_IN_SOURCE}\n{}", root.display()));
        }
    }
    Ok(())
}

fn canonical_output_parent(parent: &Path) -> Option<PathBuf> {
    let existing = parent.ancestors().find(|path| path.exists())?;
    let suffix = parent.strip_prefix(existing).ok()?;
    Some(fs::canonicalize(existing).ok()?.join(suffix))
}

const SINGLE_STREAM_FOLDER: &str = "所选格式只能压缩单个文件，不能直接压缩文件夹。请改用 TAR.GZ、ZIP 或 7z 等格式，或移除文件夹来源：";
const OUTPUT_IN_SOURCE: &str = "保存位置在来源文件夹内，压缩包会把自己打包进去。请选择来源文件夹以外的位置：";
const EARLIER_OUTPUT_IN_SOURCE: &str = "保存位置在来源文件夹内，批次中较早生成的压缩包会被后续项目打包。请选择来源文件夹以外的位置：";

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
        let chosen = plan_compression_outputs(&paths, "zip", Some(1024), &[vec![], vec![]]).unwrap();
        assert_eq!(chosen, ["Output (3).zip", "Output (4).zip"].map(|name| dir.path().join(name).display().to_string()));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn plan_rejects_batch_sources_that_creation_would_reject_midway() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        let folder = dir.path().join("project");
        fs::write(&file, b"a").unwrap();
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("x.txt"), b"x").unwrap();
        let path = |name: &str, parent: &Path| parent.join(name).display().to_string();
        let (file_source, folder_source) = (vec![file.display().to_string()], vec![folder.display().to_string()]);
        // Separate GZip: the folder source fails before the file's output is planned or written.
        let error = plan_compression_outputs(&[path("a.txt.gz", dir.path()), path("project.gz", dir.path())], "gzip", None,
            &[file_source.clone(), folder_source.clone()]).unwrap_err();
        assert!(error.starts_with(SINGLE_STREAM_FOLDER) && error.ends_with(&folder.display().to_string()), "{error}");
        assert!(plan_compression_outputs(&[path("a.txt.zst", dir.path())], "zstd", None, &[folder_source.clone()]).is_err());
        assert!(plan_compression_outputs(&[path("x.tar.gz", dir.path())], "tgz", None, &[folder_source.clone()]).is_ok());
        // Output inside a source folder, including a not-yet-created subfolder.
        let nested = folder.join("new").join("out");
        let error = plan_compression_outputs(&[path("a.zip", dir.path()), path("project.zip", &nested)], "zip", None,
            &[file_source.clone(), folder_source.clone()]).unwrap_err();
        assert!(error.starts_with(OUTPUT_IN_SOURCE), "{error}");
        assert!(!folder.join("new").exists());
        // A file source beside the output and a sibling folder are fine.
        assert!(plan_compression_outputs(&[path("both.zip", dir.path())], "zip", None,
            &[vec![file.display().to_string(), folder.display().to_string()]]).is_ok());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn staged_formats_allow_own_output_but_reject_an_earlier_output_inside_a_later_source() {
        let fixture = tempfile::tempdir().unwrap();
        let outside = fixture.path().join("outside.txt");
        let source = fixture.path().join("project");
        fs::write(&outside, b"first archive input").unwrap();
        fs::create_dir(&source).unwrap();
        let keep = source.join("keep.txt");
        fs::write(&keep, b"preserved source file").unwrap();
        let show = |path: &Path| path.display().to_string();
        let project_sources = vec![show(&source)];
        let outside_sources = vec![show(&outside)];
        for format in ["iso", "lzh"] {
            let project_output = source.join(format!("project.{format}"));
            let outside_output = source.join(format!("outside.{format}"));

            // Both staged formats can first write a single combined archive
            // inside its own source. Combining the other file is also safe.
            let combined = vec![show(&source), show(&outside)];
            assert_eq!(plan_compression_outputs(&[show(&project_output)], format, None, &[combined]).unwrap(),
                       [show(&project_output)]);

            let earlier = [show(&outside_output), show(&project_output)];
            let error = plan_compression_outputs(&earlier, format, None,
                                                 &[outside_sources.clone(), project_sources.clone()]).unwrap_err();
            assert!(error.starts_with(EARLIER_OUTPUT_IN_SOURCE), "{error}");

            // In the reverse order, no later input includes the earlier output.
            let safe_order = [show(&project_output), show(&outside_output)];
            assert_eq!(plan_compression_outputs(&safe_order, format, None,
                                                &[project_sources.clone(), outside_sources.clone()]).unwrap(),
                       safe_order);
            assert!(!project_output.exists() && !outside_output.exists());
        }
        let zip = source.join("project.zip");
        let error = plan_compression_outputs(&[show(&zip)], "zip", None, &[project_sources]).unwrap_err();
        assert!(error.starts_with(OUTPUT_IN_SOURCE), "{error}");
        assert!(!zip.exists());
        assert_eq!(fs::read(&outside).unwrap(), b"first archive input");
        assert_eq!(fs::read(&keep).unwrap(), b"preserved source file");
    }

    #[test]
    fn reused_parent_does_not_hide_a_different_output_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let first = fixture.path().join("project-copy");
        let second = fixture.path().join("project");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let input = fixture.path().join("input.txt");
        fs::write(&input, b"ordinary input").unwrap();
        let ordinary = vec![input.display().to_string()];
        let folder = vec![second.display().to_string()];
        let show = |parent: &Path, leaf| parent.join(leaf).display().to_string();

        let outputs = [show(&first, "one.zip"), show(&first, "two.zip"), show(&second, "three.zip")];
        let error = plan_compression_outputs(&outputs, "zip", None,
                                             &[ordinary.clone(), ordinary.clone(), folder.clone()]).unwrap_err();
        assert!(error.starts_with(OUTPUT_IN_SOURCE), "{error}");

        let mut outputs = [show(&first, "one.iso"), show(&first, "two.iso"), show(&first, "three.iso"), show(&first, "four.iso")];
        let sources = [ordinary.clone(), ordinary.clone(), ordinary, folder];
        assert_eq!(plan_compression_outputs(&outputs, "iso", None, &sources).unwrap(), outputs);
        outputs[2] = show(&second, "three.iso");
        let error = plan_compression_outputs(&outputs, "iso", None, &sources).unwrap_err();
        assert!(error.starts_with(EARLIER_OUTPUT_IN_SOURCE), "{error}");
        assert!(first.read_dir().unwrap().next().is_none());
        assert!(second.read_dir().unwrap().next().is_none());
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
