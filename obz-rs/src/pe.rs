//! Minimal PE parser: headers, sections, data directories, overlay offset.
//! Read-only; no code is ever executed.

use crate::util::Reader;

pub struct Section {
    pub name: String,
    pub vsize: u32,
    pub vaddr: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
}

pub struct PeImage {
    pub machine: u16,
    pub machine_name: &'static str,
    pub pe32plus: bool,
    pub timestamp: u32,
    pub sections: Vec<Section>,
    /// (rva_or_offset, size) per data directory, 16 slots.
    pub directories: Vec<(u32, u32)>,
    pub overlay_offset: usize,
}

pub fn parse(data: &[u8]) -> Result<PeImage, String> {
    if data.len() < 0x40 || &data[..2] != b"MZ" {
        return Err("not a PE image (missing MZ)".into());
    }
    let mut h = Reader::new(data);
    h.p = 0x3C;
    let pe_off = h.u32()? as usize;
    if data.len() < pe_off + 24 || &data[pe_off..pe_off + 4] != b"PE\0\0" {
        return Err("missing PE signature".into());
    }
    let mut r = Reader::new(data);
    r.p = pe_off + 4;
    let machine = r.u16()?;
    let section_count = r.u16()? as usize;
    let timestamp = r.u32()?;
    r.u32()?; // symtab pointer
    r.u32()?; // symtab count
    let opt_size = r.u16()? as usize;
    r.u16()?; // characteristics

    let opt = r.p;
    let magic = r.u16()?;
    let pe32plus = match magic {
        0x10B => false,
        0x20B => true,
        _ => return Err(format!("unknown optional header magic {magic:#x}")),
    };
    r.p = opt + 16;
    let _entry = r.u32()?;
    r.p = opt + 32;
    let _section_align = r.u32()?;
    let _file_align = r.u32()?;
    r.p = opt + 64;
    let _checksum = r.u32()?;
    r.p = opt + (92 + if pe32plus { 16 } else { 0 });
    let dir_count = (r.u32()? as usize).min(16);
    let dirs_off = opt + (96 + if pe32plus { 16 } else { 0 });

    let mut directories = Vec::with_capacity(16);
    for i in 0..16 {
        if i < dir_count {
            let mut d = Reader::new(data);
            d.p = dirs_off + i * 8;
            directories.push((d.u32()?, d.u32()?));
        } else {
            directories.push((0, 0));
        }
    }

    let mut sections = Vec::with_capacity(section_count);
    let mut raw_end = 0usize;
    let sec_off = opt + opt_size;
    for i in 0..section_count {
        let off = sec_off + i * 40;
        if off + 40 > data.len() {
            return Err("truncated section table".into());
        }
        let name = String::from_utf8_lossy(&data[off..off + 8])
            .trim_end_matches('\0')
            .to_string();
        let mut s = Reader::new(data);
        s.p = off + 8;
        let vsize = s.u32()?;
        let vaddr = s.u32()?;
        let raw_size = s.u32()?;
        let raw_offset = s.u32()?;
        raw_end = raw_end.max(raw_offset as usize + raw_size as usize);
        sections.push(Section {
            name,
            vsize,
            vaddr,
            raw_size,
            raw_offset,
        });
    }

    Ok(PeImage {
        machine,
        machine_name: match machine {
            0x014C => "x86",
            0x8664 => "x64",
            0xAA64 => "arm64",
            _ => "unknown",
        },
        pe32plus,
        timestamp,
        sections,
        directories,
        overlay_offset: raw_end.min(data.len()),
    })
}

impl PeImage {
    pub fn directory(&self, index: usize) -> Option<(u32, u32)> {
        let (rva, size) = self.directories[index];
        if size == 0 {
            None
        } else {
            Some((rva, size))
        }
    }
}
