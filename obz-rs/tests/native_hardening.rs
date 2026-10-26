use obz_core::{
    sevenz::{self, Archive, Coder, Folder},
    util::{crc32, BoundedOutput, MAX_NATIVE_OUTPUT},
    zipfmt,
};
fn vint(n: u64) -> Vec<u8> {
    let mut b = vec![0xff];
    b.extend(n.to_le_bytes());
    b
}
fn archive(header: &[u8], pack: &[u8]) -> Vec<u8> {
    let mut next = Vec::new();
    next.extend((pack.len() as u64).to_le_bytes());
    next.extend((header.len() as u64).to_le_bytes());
    next.extend(crc32(header).to_le_bytes());
    let mut data = sevenz::MAGIC.to_vec();
    data.extend([0, 4]);
    data.extend(crc32(&next).to_le_bytes());
    data.extend(next);
    data.extend(pack);
    data.extend(header);
    data
}
fn assert_bad(header: &[u8]) {
    let data = archive(header, b"");
    assert!(sevenz::parse_archive(&data).is_err());
}
#[test]
fn malicious_tiny_counts_return_errors_without_allocating() {
    let huge = vint(u64::MAX);
    for prefix in [
        vec![1, 5],
        vec![1, 4, 7, 0xb],
        vec![1, 4, 7, 0xb, 1, 0],
        vec![1, 4, 6, 0],
    ] {
        let mut h = prefix;
        h.extend(&huge);
        h.extend([0, 0, 0]);
        assert_bad(&h);
    }
}
#[test]
fn native_oversized_folder_is_rejected_before_decode() {
    let mut h = vec![1, 4, 7, 0xb, 1, 0, 1, 1, 0, 0xc];
    h.extend(vint(MAX_NATIVE_OUTPUT as u64 + 1));
    h.extend([0, 0, 0]);
    assert_bad(&h);
}
#[test]
fn native_encoded_header_has_separate_32mib_limit() {
    let mut h = vec![0x17, 6, 0, 1, 9, 1, 0, 7, 0xb, 1, 0, 1, 1, 0, 0xc];
    h.extend(vint(32 * 1024 * 1024 + 1));
    h.extend([0, 0]);
    let data = archive(&h, b"x");
    let err = sevenz::parse_archive(&data).err().unwrap();
    assert!(err.contains("limit"), "{err}");
}
#[test]
fn missing_substream_sizes_return_error_instead_of_index_panic() {
    let h = [
        1, 4, 6, 0, 1, 9, 1, 0, 7, 0xb, 1, 0, 1, 1, 0, 0xc, 1, 0, 8, 0xd, 2, 0, 0, 0,
    ];
    let data = archive(&h, b"x");
    assert!(sevenz::parse_archive(&data).is_err());
}
#[test]
fn zero_length_name_property_does_not_underflow() {
    assert_bad(&[1, 5, 1, 0x11, 0, 0, 0]);
}
fn synthetic_lzma2(output: u64, packed: usize) -> Archive {
    Archive {
        pack_offset: 0,
        pack_sizes: vec![packed],
        folders: vec![Folder {
            coders: vec![Coder {
                codec: vec![0x21],
                num_in: 1,
                num_out: 1,
                props: vec![0],
            }],
            bind_pairs: vec![],
            packed_indices: vec![0],
            unpack_sizes: vec![output],
            unpack_crc: None,
        }],
        substreams: vec![],
        entries: vec![],
        header_offset: 0,
        header_size: 0,
        encoded_header: false,
    }
}
#[test]
fn lzma2_chunk_preflight_limits_internal_accumulation() {
    let packed = [1, 0, 3, b'a', b'b', b'c', b'd', 0];
    let err = synthetic_lzma2(1, packed.len())
        .read_folder(&packed, 0)
        .unwrap_err();
    assert!(err.contains("limit"));
    assert_eq!(
        synthetic_lzma2(4, packed.len())
            .read_folder(&packed, 0)
            .unwrap(),
        b"abcd"
    );
}
#[test]
fn native_zip_inflate_is_bounded_by_declared_size() {
    let raw = vec![0; 1024 * 1024];
    let comp = miniz_oxide::deflate::compress_to_vec(&raw, 9);
    let mut data = vec![0; 30];
    data[..4].copy_from_slice(b"PK\x03\x04");
    data[8..10].copy_from_slice(&8u16.to_le_bytes());
    data.extend(&comp);
    let e = zipfmt::ZipEntry {
        name: "bomb".into(),
        size: 1,
        comp_size: comp.len() as u64,
        method: 8,
        crc: crc32(&raw),
        local_offset: 0,
    };
    assert!(zipfmt::extract_entry(&data, 0, &e).is_err());
    let e = zipfmt::ZipEntry {
        size: MAX_NATIVE_OUTPUT as u64 + 1,
        ..e
    };
    assert!(zipfmt::extract_entry(&data, 0, &e)
        .unwrap_err()
        .contains("limit"));
}
#[test]
fn bounded_writer_rejects_excess_before_reserving() {
    use std::io::Write;
    let mut out = BoundedOutput::new(2);
    assert!(out.write_all(b"abc").is_err());
    assert!(out.bytes.is_empty());
}
#[test]
fn structured_header_byte_mutations_do_not_panic() {
    let base = [
        1, 4, 6, 0, 1, 9, 1, 0, 7, 0xb, 1, 0, 1, 1, 0, 0xc, 1, 0, 0, 5, 1, 0x11, 5, 0, b'a', 0, 0,
        0, 0, 0,
    ];
    for i in 0..base.len() {
        for value in [0, 0x7f, 0x80, 0xff] {
            let mut h = base;
            h[i] = value;
            let data = archive(&h, b"x");
            let _ = sevenz::parse_archive(&data);
        }
    }
}
