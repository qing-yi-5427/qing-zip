//! ACE archive decryption using Blowfish
//!
//! ACE uses Blowfish in CBC mode for encryption.
//! ACE uses non-standard SHA-0-like derivation and modified Blowfish S-boxes.

use super::ace_blowfish::AceBlowfish;


/// Blowfish block size in bytes
const BLOCK_SIZE: usize = 8;

// ACE key derivation and little-endian block layout are based on acefile,
// Copyright (c) 2017-2026 Daniel Roethlisberger and contributors, BSD-2-Clause.
// See crate LICENSE-ACEFILE.md. ACE uses a SHA-0-like 160-bit derivation,
// not standard SHA-1 truncated to 128 bits.
pub fn derive_key(password: &str) -> [u8; 20] {
    let pwd=&password.as_bytes()[..password.len().min(50)];
    let mut block=[0u8;64];block[..pwd.len()].copy_from_slice(pwd);block[pwd.len()]=0x80;
    let mut words=[0u32;80];
    for (i,word) in words[..15].iter_mut().enumerate(){*word=u32::from_le_bytes(block[i*4..i*4+4].try_into().unwrap());}
    words[15]=(pwd.len() as u32)*8;
    for i in 16..80 {words[i]=words[i-16]^words[i-14]^words[i-8]^words[i-3];}
    let initial=[0x67452301u32,0xefcdab89,0x98badcfe,0x10325476,0xc3d2e1f0];
    let [mut a,mut b,mut c,mut d,mut e]=initial;
    for (i,&word) in words.iter().enumerate(){let (f,k)=match i {0..20=>((b&c)|(!b&d),0x5a827999u32),20..40=>(b^c^d,0x6ed9eba1),40..60=>((b&c)|(b&d)|(c&d),0x8f1bbcdc),_=>(b^c^d,0xca62c1d6)};let t=a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(word).wrapping_add(k);e=d;d=c;c=b.rotate_left(30);b=a;a=t;}
    let mut key=[0u8;20];for (i,v) in [a,b,c,d,e].into_iter().enumerate(){key[i*4..i*4+4].copy_from_slice(&v.wrapping_add(initial[i]).to_be_bytes());}key
}

/// Decrypt data using Blowfish in CBC mode
///
/// ACE uses a zero IV for CBC mode.
pub fn decrypt_cbc(data: &[u8], key: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }

    // ACE-modified Blowfish with 160-bit key and little-endian block words
    let cipher=AceBlowfish::new(key);

    let mut result = data.to_vec();
    let mut prev_block = [0u8; BLOCK_SIZE]; // Zero IV

    // Process each block
    for chunk in result.as_chunks_mut::<BLOCK_SIZE>().0 {
        let encrypted_block: [u8; BLOCK_SIZE] = *chunk;

        // Decrypt block
        let words=[u32::from_le_bytes(chunk[..4].try_into().unwrap()),u32::from_le_bytes(chunk[4..].try_into().unwrap())];
        let plain=cipher.decrypt(words);let mut block=[0u8;8];block[..4].copy_from_slice(&plain[0].to_le_bytes());block[4..].copy_from_slice(&plain[1].to_le_bytes());

        // XOR with previous ciphertext (CBC mode)
        for (i, byte) in chunk.iter_mut().enumerate() {
            *byte = block[i] ^ prev_block[i];
        }

        prev_block = encrypted_block;
    }

    result
}

/// Decrypt ACE file data
///
/// Decrypts the compressed data and returns the decrypted bytes.
pub fn decrypt_ace_data(encrypted_data: &[u8], password: &str) -> Vec<u8> {
    let key = derive_key(password);
    decrypt_cbc(encrypted_data, &key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_key() {
        let key = derive_key("test");
        assert_eq!(key.len(), 20);
        assert_eq!(derive_key("123456789"), [0xb7,0xe,0xcb,0x9c,0xc6,0x3c,0x99,0xe5,0xf1,0xe5,0x0,0x85,0x4e,0x2,0x46,0xfe,0x35,0x71,0x36,0x9e]);
    }

    #[test]
    fn acefile_reference_cbc_vector() {
        assert_eq!(decrypt_ace_data(&[0xff;8],"123456789"), [0xb7,0x77,0x46,0x40,0x35,0x2e,0x65,0x72]);
    }
    #[test]
    fn test_decrypt_empty() {
        let result = decrypt_ace_data(&[], "test");
        assert!(result.is_empty());
    }
}
