use super::ShellDragDirectory;
use crate::engine_stream::PinnedSource;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

pub(super) struct UpdateList {
    // Release the no-delete reader before removing the directory.
    _source: PinnedSource,
    _directory: ShellDragDirectory,
}

/// Keep short commands unchanged. Long selections use the console's UTF-8
/// list format so the archive edit still runs as a single checked operation.
pub(super) fn append(
    args: &mut Vec<String>,
    archive: &Path,
    names: Vec<String>,
) -> Result<Option<UpdateList>, String> {
    let archive = archive.to_str().ok_or("invalid archive path")?;
    let list_bytes: u64 = names.iter().map(|name| name.len() as u64 + 3).sum();
    if list_bytes >= (1u64 << 31) - 32 {
        return Err("archive update list exceeds the engine limit".into());
    }
    // This leaves room for executable, policy switches and Windows quoting
    // before the 32,767 UTF-16 unit process command-line boundary.
    let length = args.iter().map(|s| s.len() + 3).sum::<usize>() + list_bytes as usize;
    if length.saturating_add(archive.len()) <= 12_000 {
        args.extend(["--".into(), archive.to_owned()]);
        args.extend(names);
        return Ok(None);
    }
    if !Path::new(archive).is_absolute() {
        return Err("archive update requires an absolute path".into());
    }
    let directory = ShellDragDirectory::create()?;
    let path = directory.path().join("members.txt");
    {
        let mut writer = BufWriter::new(File::create_new(&path).map_err(|e| e.to_string())?);
        for name in &names {
            if name.is_empty() || name.contains(['\r', '\n', '\0']) {
                return Err("invalid name in archive update".into());
            }
            // The console trims outer whitespace and strips one pair of quotes.
            // Wrapping every name preserves leading spaces and a leading BOM.
            for part in [b"\"".as_slice(), name.as_bytes(), b"\"\n"] {
                writer.write_all(part).map_err(|e| e.to_string())?;
            }
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    let mut source = crate::engine_stream::pin(&path)?;
    if source.metadata().map_err(|e| e.to_string())?.len() != list_bytes {
        return Err("archive update list changed".into());
    }
    let mut reader = BufReader::new(&mut *source);
    let mut line = Vec::new();
    for name in &names {
        line.resize(name.len() + 3, 0);
        reader.read_exact(&mut line).map_err(|e| e.to_string())?;
        if line[0] != b'"' || &line[1..1 + name.len()] != name.as_bytes() || !line.ends_with(b"\"\n") {
            return Err("archive update list changed".into());
        }
    }
    // -- also disables @list parsing. The archive is an absolute Windows path
    // and all member text is in the verified list, never in an option position.
    args.extend(["-scsUTF-8".into(), archive.to_owned(), format!("@{}", path.display())]);
    Ok(Some(UpdateList { _source: source, _directory: directory }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_update_lists_keep_exact_lines_and_remain_read_only_until_drop() {
        let names = [" @items", "-a&b", "folder/leaf", "\u{feff}name"].repeat(1000)
            .iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut args = vec!["d".to_string(), "-spd".to_string()];
        let guard = append(&mut args, Path::new(r"C:\input.zip"), names.clone()).unwrap().unwrap();
        let path = Path::new(args.last().unwrap().strip_prefix('@').unwrap()).to_owned();
        let expected = names.iter().map(|name| format!("\"{name}\"\n")).collect::<String>();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert!(File::options().write(true).open(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(guard);
        assert!(!path.parent().unwrap().exists());
    }
}
