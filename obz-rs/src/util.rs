//! Byte-level helpers: cursor reader and CRC-32 (IEEE).

pub struct Reader<'a> {
    pub d: &'a [u8],
    pub p: usize,
}

impl<'a> Reader<'a> {
    pub fn new(d: &'a [u8]) -> Self {
        Reader { d, p: 0 }
    }

    pub fn read(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.p.checked_add(n).ok_or("input range overflow")?;
        let out = self.d.get(self.p..end).ok_or("truncated input")?;
        self.p = end;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.read(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        let b = self.read(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        let b = self.read(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, String> {
        let b = self.read(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// 7z variable-length UINT64 encoding.
    pub fn varint(&mut self) -> Result<u64, String> {
        let first = self.u8()?;
        let mut mask: u8 = 0x80;
        let mut value: u64 = 0;
        for i in 0..8u32 {
            if first & mask != 0 {
                value |= (self.u8()? as u64) << (8 * i);
                mask >>= 1;
            } else {
                value |= ((first & (mask - 1)) as u64) << (8 * i);
                return Ok(value);
            }
        }
        Ok(value)
    }

    pub fn bytes_left(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
}

/// CRC-32 (IEEE 802.3), reflected, init/xorout 0xFFFFFFFF.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: [u32; 256] = build_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// Native archive decoding is intentionally bounded; generic large archives use
/// the streaming multi-format engine instead.
pub const MAX_NATIVE_OUTPUT: usize = 512 * 1024 * 1024;
pub struct BoundedOutput {
    pub bytes: Vec<u8>,
    limit: usize,
}
impl BoundedOutput {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}
impl std::io::Write for BoundedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let end = self
            .bytes
            .len()
            .checked_add(buf.len())
            .ok_or_else(|| std::io::Error::other("decoded size overflow"))?;
        if end > self.limit {
            return Err(std::io::Error::other(
                "native decoded output limit exceeded",
            ));
        }
        self.bytes
            .try_reserve(buf.len())
            .map_err(|_| std::io::Error::other("decoded allocation failed"))?;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
