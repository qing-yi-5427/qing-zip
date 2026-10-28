//! Choose the outer destination without flattening or changing archive entries.
use crate::engine::{self, EngineOptions, Entry};
use std::path::{Path, PathBuf};

fn archive_name(path: &str) -> Result<String,String> {
    let mut name=Path::new(path).file_name().and_then(|s|s.to_str()).ok_or("invalid archive filename")?.to_owned();
    if let Some((base,tail))=name.rsplit_once('.') {
        if tail.len()>=3 && tail.bytes().all(|b|b.is_ascii_digit()){name=base.to_owned();}
    }
    let lower=name.to_ascii_lowercase();
    for suffix in [".tar.gz",".tar.bz2",".tar.xz",".tar.zst"] {
        if lower.ends_with(suffix){name.truncate(name.len()-suffix.len());return Ok(name);}
    }
    name=Path::new(&name).file_stem().and_then(|s|s.to_str()).ok_or("invalid archive filename")?.to_owned();
    if let Some(index)=name.to_ascii_lowercase().rfind(".part") {
        let part=&name[index+5..];
        if !part.is_empty()&&part.bytes().all(|b|b.is_ascii_digit()){name.truncate(index);}
    }
    Ok(name)
}

pub fn destination(archive: &str, base: &str, entries: &[Entry], opts: &EngineOptions) -> Result<String,String> {
    let stem=archive_name(archive)?;
    let folder=crate::extract::relative_path(&stem)?;
    let mut already_wrapped=true;
    let mut matched=false;
    for entry in entries {
        let safe=engine::validate_name(&entry.name)?;
        if !engine::selected(&entry.name, opts) { continue; }
        matched=true;
        let mut parts=safe.components();
        let top=parts.next().ok_or("invalid archive entry")?.as_os_str().to_string_lossy();
        // An implicit directory (only child entries) counts as a top-level folder too.
        if !top.eq_ignore_ascii_case(&stem) || (!entry.is_dir && parts.next().is_none()){already_wrapped=false;}
    }
    if !matched && !opts.selected.is_empty() { return Err("selected archive entries were not found".into()); }
    already_wrapped &= matched;
    Ok(if already_wrapped {PathBuf::from(base)} else {PathBuf::from(base).join(folder)}.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(name:&str,is_dir:bool)->Entry {Entry{name:name.into(),is_dir,size:0,packed_size:0,crc:String::new(),encrypted:false,modified:String::new(),method:String::new(),comment:String::new(),attributes:String::new()}}
    #[test] fn top_level_layouts(){
        let base="output";
        assert_eq!(destination("bundle.zip",base,&[e("bundle/a.txt",false)], &EngineOptions::default()).unwrap(),base);
        assert_eq!(destination("bundle.zip",base,&[e("BUNDLE/",true),e("BUNDLE\\a.txt",false)], &EngineOptions::default()).unwrap(),base);
        for entries in [vec![e("a.txt",false),e("b.txt",false)],vec![e("bundle",false)],vec![e("other/a.txt",false)],vec![e("bundle/a.txt",false),e("readme.txt",false)]] {
            assert_eq!(PathBuf::from(destination("bundle.zip",base,&entries, &EngineOptions::default()).unwrap()),Path::new(base).join("bundle"));
        }
        assert!(destination("bundle.zip",base,&[e("../outside.txt",false)], &EngineOptions::default()).is_err());
    }
    #[test] fn compound_and_volume_names(){
        for name in ["bundle.tar.gz","bundle.tar.xz","bundle.7z.001","bundle.zip.001","bundle.part01.rar"] {assert_eq!(archive_name(name).unwrap(),"bundle");}
    }
    fn opts(selected: &[&str]) -> EngineOptions {
        EngineOptions { selected: selected.iter().map(|s| s.to_string()).collect(), ..EngineOptions::default() }
    }
    fn check(archive: &str, entries: &[Entry], selection: &[&str], expected: &str) {
        assert_eq!(PathBuf::from(destination(archive, "output", entries, &opts(selection)).unwrap()), PathBuf::from(expected));
    }
    #[test]
    fn selected_single_and_multiple_files_use_only_their_layout() {
        let entries = [e("bundle/a.txt", false), e("bundle/b.txt", false), e("readme.txt", false), e("other/c.txt", false)];
        check("bundle.zip", &entries, &["bundle/a.txt"], "output");
        check("bundle.zip", &entries, &["bundle/a.txt", "bundle/b.txt"], "output");
        check("bundle.zip", &entries, &["readme.txt"], "output/bundle");
        check("bundle.zip", &entries, &["bundle/a.txt", "other/c.txt"], "output/bundle");
        check("bundle.zip", &entries, &[], "output/bundle");
    }
    #[test]
    fn selected_directory_includes_descendants_and_mixed_directories() {
        let entries = [e("bundle/", true), e("bundle/a.txt", false), e("bundle/sub/b.txt", false), e("other/c.txt", false)];
        check("bundle.zip", &entries, &["bundle"], "output");
        check("bundle.zip", &entries, &["bundle\\sub/"], "output");
        check("bundle.zip", &entries, &["bundle/", "other"], "output/bundle");
        check("bundle.zip", &entries, &["bundle/"], "output");
        check("bundle.zip", &[e("bundle/", true)], &["bundle/"], "output");
    }
    #[test]
    fn empty_and_invalid_selections() {
        let entries = [e("bundle/a.txt", false), e("readme.txt", false)];
        check("bundle.zip", &entries, &[], "output/bundle");
        check("bundle.zip", &[], &[], "output/bundle");
        assert_eq!(destination("bundle.zip", "output", &entries, &opts(&["missing"])).unwrap_err(), "selected archive entries were not found");
        assert_eq!(destination("bundle.zip", "output", &[], &opts(&["missing"])).unwrap_err(), "selected archive entries were not found");
        check("bundle.zip", &entries, &["missing", "bundle/a.txt"], "output");
    }
    #[test]
    fn selected_entries_work_with_compound_and_volume_names() {
        let entries = [e("bundle/a.txt", false), e("readme.txt", false)];
        for archive in ["bundle.tar.gz", "bundle.tar.bz2", "bundle.tar.xz", "bundle.tar.zst", "bundle.7z.001", "bundle.zip.001", "bundle.part01.rar"] {
            check(archive, &entries, &["bundle/a.txt"], "output");
            check(archive, &entries, &["readme.txt"], "output/bundle");
        }
    }
    #[test]
    fn unsafe_selected_and_unselected_paths_are_rejected() {
        for bad in ["../outside.txt", "/absolute.txt", "C:\\outside.txt", "bundle/../../outside.txt"] {
            let entries = [e("bundle/a.txt", false), e(bad, false)];
            assert!(destination("bundle.zip", "output", &entries, &opts(&[bad])).is_err(), "{bad}");
            assert!(destination("bundle.zip", "output", &entries, &opts(&["bundle/a.txt"])).is_err(), "{bad}");
        }
    }

}
