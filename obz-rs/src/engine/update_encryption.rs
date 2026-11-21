//! Verify the original password before any file can be replaced, and verify
//! encryption metadata on the proposal before publishing it.
use super::*;

pub(super) struct Protection {
    original: HashMap<String, u8>,
    method: Option<(&'static str, u8)>,
    headers: bool,
    password: bool,
}

fn strength(entry: &Entry) -> u8 {
    if !entry.encrypted { return 0; }
    for (method, rank) in [("AES-256", 4), ("AES-192", 3), ("AES-128", 2)] {
        if entry.method.split_whitespace().any(|part| part == method) { return rank; }
    }
    1
}

fn name(entry: &Entry) -> String {
    path_key(Path::new(&entry.name)).trim_end_matches('/').to_owned()
}

impl Protection {
    pub(super) fn read(path: &Path, opts: &EngineOptions) -> Result<(Self, Vec<Entry>), String> {
        let path_text = path.to_str().ok_or("invalid archive path")?;
        let mut listing = opts.clone();
        listing.selected.clear();
        let entries = list(path_text, &listing)?;
        let mut archive = crate::engine_stream::pin(path)?;
        let mut magic = [0; 6];
        let count = archive.read(&mut magic).map_err(|e| e.to_string())?;
        let zip = count >= 4 && matches!(&magic[..4], b"PK\x03\x04" | b"PK\x05\x06")
            || entries.iter().any(|entry| strength(entry) >= 2);
        let seven_zip = magic == *b"7z\xBC\xAF\x27\x1C";
        let password = opts.password.is_some();
        let headers = if !zip && password {
            let mut without_password = listing.clone();
            without_password.password = None;
            let locked = list(path_text, &without_password).is_err();
            control(opts).check()?;
            locked || opts.header_encryption == Some(true)
        } else {
            false
        };
        if opts.header_encryption == Some(true) && !(seven_zip && password) {
            return Err("加密文件名需要 7z 格式和密码".into());
        }
        if entries.iter().any(|entry| entry.encrypted) {
            if !password { return Err("请提供现有压缩包的密码".into()); }
            test(path_text, &listing)?;
        }

        let methods = [("ZipCrypto", 1), ("AES128", 2), ("AES192", 3), ("AES256", 4)];
        let old_strength = entries.iter().map(strength).max().unwrap_or(0);
        let method = match opts.encryption.as_deref() {
            Some(value) if password && zip => {
                let method = methods.into_iter().find(|(method, _)| method.eq_ignore_ascii_case(value))
                    .ok_or("ZIP 加密方式无效")?;
                if method.1 < old_strength { return Err("所选加密方式低于现有 ZIP 的加密强度".into()); }
                Some(method)
            }
            Some(value) if password && seven_zip && value.eq_ignore_ascii_case("aes256") => None,
            Some(_) => return Err("所选加密方式要求兼容的压缩格式和密码".into()),
            None if zip && password => Some(methods[usize::from(if old_strength == 0 { 4 } else { old_strength } - 1)]),
            None => None,
        };
        let mut original = HashMap::new();
        if password {
            for entry in entries.iter().filter(|entry| !entry.is_dir) {
                if original.insert(name(entry), strength(entry)).is_some() {
                    return Err("压缩包包含重名文件，无法核验更新后的加密状态".into());
                }
            }
        }
        Ok((Self { original, method, headers, password }, entries))
    }

    pub(super) fn add_switches(&self, args: &mut Vec<String>) {
        if let Some((method, _)) = self.method { args.push(format!("-mem={method}")); }
        if self.headers { args.push("-mhe=on".into()); }
    }

    pub(super) fn verify(&self, path: &Path, opts: &EngineOptions) -> Result<(), String> {
        if !self.password { return Ok(()); }
        let mut listing = opts.clone();
        listing.selected.clear();
        let path = path.to_str().ok_or("invalid archive path")?;
        if self.headers {
            let mut without_password = listing.clone();
            without_password.password = None;
            if list(path, &without_password).is_ok() { return Err("更新后压缩包文件名未加密".into()); }
            control(opts).check()?;
        }
        let mut seen = HashSet::new();
        for entry in list(path, &listing)?.into_iter().filter(|entry| !entry.is_dir) {
            let key = name(&entry);
            let rank = match self.original.get(&key) {
                Some(&rank) => rank,
                // 7z may represent empty files outside an encrypted data block.
                None if entry.size == 0 && self.method.is_none() => 0,
                None => self.method.map_or(1, |(_, rank)| rank),
            };
            if strength(&entry) < rank || !seen.insert(key) {
                return Err(format!("无法核验更新后的文件加密状态：{}", entry.name));
            }
        }
        if opts.update_mode != UpdateMode::Sync
            && self.original.iter().any(|(name, &rank)| rank > 0 && !seen.contains(name))
        {
            return Err("更新后压缩包缺少原有加密文件".into());
        }
        Ok(())
    }
}
