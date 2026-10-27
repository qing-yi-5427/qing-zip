//! Choose the outer destination without flattening or changing archive entries.
use crate::engine::{self, Entry};
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

pub fn destination(archive: &str, base: &str, entries: &[Entry]) -> Result<String,String> {
    let stem=archive_name(archive)?;
    let folder=crate::extract::relative_path(&stem)?;
    let mut already_wrapped=!entries.is_empty();
    for entry in entries {
        let safe=engine::validate_name(&entry.name)?;
        let mut parts=safe.components();
        let top=parts.next().ok_or("invalid archive entry")?.as_os_str().to_string_lossy();
        // An implicit directory (only child entries) counts as a top-level folder too.
        if !top.eq_ignore_ascii_case(&stem) || (!entry.is_dir && parts.next().is_none()){already_wrapped=false;}
    }
    Ok(if already_wrapped {PathBuf::from(base)} else {PathBuf::from(base).join(folder)}.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(name:&str,is_dir:bool)->Entry {Entry{name:name.into(),is_dir,size:0,packed_size:0,crc:String::new(),encrypted:false,modified:String::new(),method:String::new(),comment:String::new(),attributes:String::new()}}
    #[test] fn top_level_layouts(){
        let base="output";
        assert_eq!(destination("bundle.zip",base,&[e("bundle/a.txt",false)]).unwrap(),base);
        assert_eq!(destination("bundle.zip",base,&[e("BUNDLE/",true),e("BUNDLE\\a.txt",false)]).unwrap(),base);
        for entries in [vec![e("a.txt",false),e("b.txt",false)],vec![e("bundle",false)],vec![e("other/a.txt",false)],vec![e("bundle/a.txt",false),e("readme.txt",false)]] {
            assert_eq!(PathBuf::from(destination("bundle.zip",base,&entries).unwrap()),Path::new(base).join("bundle"));
        }
        assert!(destination("bundle.zip",base,&[e("../outside.txt",false)]).is_err());
    }
    #[test] fn compound_and_volume_names(){
        for name in ["bundle.tar.gz","bundle.tar.xz","bundle.7z.001","bundle.zip.001","bundle.part01.rar"] {assert_eq!(archive_name(name).unwrap(),"bundle");}
    }
}
