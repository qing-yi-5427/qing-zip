//! Bandizip setup package model: PE stub + 7z payload + ZIP UI + certificate.
//! Every offset is discovered by parsing, never hard-coded.

use crate::pe::{self, PeImage};
use crate::sevenz::{self, Archive};
use crate::zipfmt::{self, ZipEntry};

pub struct CertBlock {
    pub offset: usize,
    pub length: u32,
    pub revision: u16,
    pub cert_type: u16,
}

pub struct SetupPackage<'a> {
    pub data: &'a [u8],
    pub pe: PeImage,
    pub sevenz_offset: usize,
    pub archive: Archive,
    pub zip_offset: usize,
    pub zip_end: usize,
    pub ui_entries: Vec<ZipEntry>,
    pub cert: Option<CertBlock>,
}

pub fn open(data: &[u8]) -> Result<SetupPackage<'_>, String> {
    let pe = pe::parse(data)?;

    let search_from = pe.overlay_offset.saturating_sub(64);
    let sevenz_offset = find(&data[search_from..], sevenz::MAGIC)
        .map(|i| search_from + i)
        .ok_or_else(|| "no embedded 7z payload found".to_string())?;
    let archive = sevenz::parse_archive(&data[sevenz_offset..])?;

    let pack_end = archive.pack_offset + archive.pack_sizes.iter().sum::<usize>();
    let header_end = archive.header_offset + archive.header_size;
    let sevenz_end = sevenz_offset + pack_end.max(header_end);

    let zip_offset = find(
        data.get(sevenz_end..).ok_or("7z range exceeds input")?,
        b"PK\x03\x04",
    )
    .map(|i| sevenz_end + i)
    .ok_or_else(|| "no installer UI zip found".to_string())?;
    let eocd = find_last(data, b"PK\x05\x06").ok_or_else(|| "zip EOCD not found")?;
    let comment = data.get(eocd + 20..eocd + 22).ok_or("truncated ZIP EOCD")?;
    let comment_len = u16::from_le_bytes([comment[0], comment[1]]) as usize;
    let zip_end = eocd + 22 + comment_len;

    let ui_entries = zipfmt::read_central_dir(data, zip_offset, zip_end)?;

    let cert = match pe.directory(4) {
        Some((off, _size)) => {
            let off = off as usize;
            if off + 8 <= data.len() {
                let length =
                    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
                let revision = u16::from_le_bytes([data[off + 4], data[off + 5]]);
                let cert_type = u16::from_le_bytes([data[off + 6], data[off + 7]]);
                Some(CertBlock {
                    offset: off,
                    length,
                    revision,
                    cert_type,
                })
            } else {
                None
            }
        }
        None => None,
    };

    Ok(SetupPackage {
        data,
        pe,
        sevenz_offset,
        archive,
        zip_offset,
        zip_end,
        ui_entries,
        cert,
    })
}

pub fn extract_payload(pkg: &SetupPackage<'_>, dest: &str) -> Result<usize, String> {
    crate::extract::sevenz(&pkg.archive, pkg.data_sevenz(), dest)
}

pub fn extract_ui(pkg: &SetupPackage<'_>, dest: &str) -> Result<usize, String> {
    crate::extract::zip(pkg.data, pkg.zip_offset, &pkg.ui_entries, dest)
}

impl<'a> SetupPackage<'a> {
    pub fn data_sevenz(&self) -> &'a [u8] {
        &self.data[self.sevenz_offset..]
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|&i| &hay[i..i + needle.len()] == needle)
}
