use super::{choose_algorithms, Algorithm, EngineOptions, HashInput};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Single-file callers retain the original response. Batches keep input order,
/// including repeated paths, and publish results only when all files succeed.
pub fn checksum_files(
    paths: &[String],
    names: &[String],
    opts: &EngineOptions,
) -> Result<Value, String> {
    if paths.is_empty() {
        return Err("checksum requires one or more files".into());
    }
    let control = opts.control.clone().unwrap_or_default();
    control.check()?;
    control.set("checksum", Some(0), "Calculating file checksums");
    // A child operation can report 100% for its own input. Publish batch
    // progress separately while sharing immediate cancellation with the child.
    let child_options = EngineOptions {
        control: Some(super::JobControl {
            cancelled: control.cancelled.clone(),
            ..Default::default()
        }),
        ..opts.clone()
    };
    let progress = |done: usize| {
        control.set(
            "checksum",
            Some((done as u128 * 100 / paths.len() as u128) as u8),
            "Calculating file checksums",
        )
    };
    let mut results = Vec::with_capacity(paths.len());
    if super::CommonSelection::from_names(names).is_some() {
        for path in paths {
            results.push(super::checksum_selected(path, names, &child_options)?);
            progress(results.len());
        }
    } else {
        let chosen = choose_algorithms(names, &child_options)?;
        // Bound simultaneously held source handles and the engine's table size.
        for group in paths.chunks(128) {
            control.check()?;
            let pinned = group
                .iter()
                .map(|path| {
                    control.check()?;
                    HashInput::open(path)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let hashes = hash_group(&pinned, &chosen, &child_options)?;
            for (path, input) in group.iter().zip(&pinned) {
                input.verify()?;
                let row = hashes.get(&input.absolute).ok_or("missing checksum row")?;
                results.push(json!({"path":path,"size":input.before.len(),"hashes":row}));
            }
            progress(results.len());
        }
    }
    control.check()?;
    control.set("complete", Some(100), "File checksums ready");
    Ok(json!({"checksums":results}))
}

type HashRows = HashMap<String, BTreeMap<String, String>>;

fn hash_group(
    inputs: &[HashInput],
    chosen: &[Algorithm],
    opts: &EngineOptions,
) -> Result<HashRows, String> {
    use std::io::{Read, Write};
    let expected: BTreeMap<_, _> = inputs
        .iter()
        .map(|input| (input.absolute.as_str(), input.before.len()))
        .collect();
    // A list file also works when long Windows paths exceed the command-line limit.
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut names = String::new();
    for name in expected.keys() {
        names.push_str(name);
        names.push('\n');
    }
    file.write_all(names.as_bytes())
        .map_err(|e| e.to_string())?;
    let list_path = file.into_temp_path();
    let mut guard = crate::engine_stream::pin(&list_path)?;
    let mut contents = String::new();
    (&mut *guard)
        .take(names.len() as u64 + 1)
        .read_to_string(&mut contents)
        .map_err(|e| e.to_string())?;
    if contents != names {
        return Err("checksum input list changed".into());
    }
    let mut args: Vec<String> = [
        "h",
        "-spf",
        "-ssc",
        "-scsUTF-8",
        "-sccUTF-8",
        "-bsp2",
        "-spd",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(chosen.iter().map(|a| format!("-scrc{}", a.name)));
    args.push(format!("@{}", list_path.display()));
    let bytes = crate::engine::run_hash(&args, opts)?;
    parse_table(
        std::str::from_utf8(&bytes).map_err(|_| "invalid checksum output encoding")?,
        chosen,
        &expected,
    )
}

fn parse_table(
    text: &str,
    chosen: &[Algorithm],
    expected: &BTreeMap<&str, u64>,
) -> Result<HashRows, String> {
    let mut lines = text.lines();
    let header = lines
        .find(|line| line.split_whitespace().rev().take(2).eq(["Name", "Size"]))
        .ok_or("missing engine checksum table")?;
    let mut fields: Vec<_> = header.split_whitespace().collect();
    fields.truncate(fields.len() - 2);
    if fields.len() != chosen.len() {
        return Err("unexpected checksum columns".into());
    }
    let columns: Vec<_> = fields
        .iter()
        .map(|name| {
            chosen
                .iter()
                .find(|a| &a.name == name)
                .ok_or("unknown checksum column")
        })
        .collect::<Result<_, _>>()?;
    let separator = lines.next().ok_or("missing checksum table separator")?;
    if separator.is_empty() || !separator.bytes().all(|c| c == b'-' || c == b' ') {
        return Err("invalid checksum table separator".into());
    }
    let mut rows = HashMap::new();
    for line in lines {
        if line == separator {
            if rows.len() != expected.len() {
                return Err("incomplete checksum table".into());
            }
            return Ok(rows);
        }
        let mut rest = line;
        let mut hashes = BTreeMap::new();
        for algorithm in &columns {
            let (digest, next) = rest
                .trim_start()
                .split_once(' ')
                .ok_or("missing file digest")?;
            if digest.len() != algorithm.bytes * 2
                || !digest.bytes().all(|c| c.is_ascii_hexdigit())
                || hashes
                    .insert(algorithm.name.clone(), digest.to_ascii_lowercase())
                    .is_some()
            {
                return Err("invalid file digest".into());
            }
            rest = next;
        }
        let (size, name) = rest
            .trim_start()
            .split_once(' ')
            .ok_or("missing checksum file name")?;
        let size: u64 = size.parse().map_err(|_| "invalid checksum file size")?;
        let name = name.trim_start();
        if expected.get(name) != Some(&size) || rows.insert(name.to_string(), hashes).is_some() {
            return Err("unexpected or duplicate checksum file".into());
        }
    }
    Err("unterminated checksum table".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_parser_requires_the_exact_files_and_columns() {
        let expected = BTreeMap::from([("C:\\work\\a b", 7)]);
        let chosen = [Algorithm {
            name: "CRC32".into(),
            bytes: 4,
        }];
        let table = "CRC32    Size  Name\n-------- ---- ------\nABCDEF01    7  C:\\work\\a b\n-------- ---- ------";
        assert_eq!(
            parse_table(table, &chosen, &expected).unwrap()["C:\\work\\a b"]["CRC32"],
            "abcdef01"
        );
        for invalid in [
            table.replace("ABCDEF01", "ABCDEF0g"),
            table.replace("    7  ", "    8  "),
            table.replace("a b", "a"),
            table.replace("CRC32", "CRC64"),
            table.replace("ABCDEF01", "ABCDEF01    7  C:\\work\\a b\nABCDEF01"),
            table.lines().take(3).collect::<Vec<_>>().join("\n"),
        ] {
            assert!(parse_table(&invalid, &chosen, &expected).is_err());
        }
    }
}
