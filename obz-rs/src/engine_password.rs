//! Password transport for the unmodified 7-Zip/NanaZip console engines.
//!
//! Read commands must omit `-p`: bare `-p` means an explicitly empty password
//! there. Update commands use bare `-p` to request stdin, never `-p{secret}`.
//! `-sccUTF-8` controls stdin decoding as well as console output. Both callbacks
//! cache the password; updating an encrypted archive can read once to open and
//! once to write. Neither callback requests password confirmation.
use std::{
    io::{self, Write},
    process::Child,
    sync::atomic::{compiler_fence, Ordering},
    thread::{self, JoinHandle},
};

pub fn validate(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password must not be empty".into());
    }
    if password.contains(['\0', '\r', '\n']) {
        return Err("password contains invalid characters".into());
    }
    // Windows CRT text-mode stdin treats Ctrl+Z as EOF. Accepting it would
    // silently create an archive using only the password prefix.
    if password.contains('\u{1a}') {
        return Err("password contains Ctrl+Z, which the Windows engine cannot read safely".into());
    }
    Ok(())
}

pub(crate) fn needs_prompt_switch(command: &str) -> bool {
    matches!(command, "a" | "u" | "d" | "rn")
}

// This is only the temporary transport copy. EngineOptions owns the original.
struct SecretLine(Vec<u8>);
impl Drop for SecretLine {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // Prevent the compiler from removing the wipe before deallocation.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

/// Owns the only stdin writer. The caller must reap/kill the child before
/// calling `finish`, including on cancellation and process-wait failures.
/// Writing on a separate thread keeps cancellation responsive even for a very
/// long password or a command (such as ZIP listing) that never reads stdin.
pub struct PasswordInput(JoinHandle<io::Result<()>>);

impl PasswordInput {
    pub fn spawn(child: &mut Child, password: &str) -> Result<Self, String> {
        validate(password)?;
        let mut stdin = child.stdin.take().ok_or("engine password pipe missing")?;
        let mut line = SecretLine(Vec::with_capacity(password.len() + 1));
        line.0.extend_from_slice(password.as_bytes());
        line.0.push(b'\n');
        thread::Builder::new()
            .name("archive-password-input".into())
            .spawn(move || {
                // One read callback and one write callback at most: qing-zip
                // invokes one archive per engine process, without stdin data.
                for _ in 0..2 {
                    if let Err(error) = stdin.write_all(&line.0) {
                        // Listing unencrypted headers may exit without reading.
                        if error.kind() == io::ErrorKind::BrokenPipe {
                            return Ok(());
                        }
                        return Err(error);
                    }
                }
                Ok(())
            })
            .map(Self)
            .map_err(|_| "cannot start engine password input".into())
    }

    pub fn finish(self) -> Result<(), String> {
        match self.0.join() {
            Ok(Ok(())) => Ok(()),
            _ => Err("cannot send password to archive engine".into()),
        }
    }
}

/// `List.cpp` hard-codes its password prompt to stdout, independent of `-bso`.
/// Remove only that exact leading protocol prefix, once. Never filter lines in
/// the listing body: ambiguous filenames/metadata must still fail validation.
pub(crate) fn listing_payload(bytes: &[u8]) -> &[u8] {
    const CRLF: &[u8] = b"\r\nEnter password (will not be echoed):\r\n";
    const LF: &[u8] = b"\nEnter password (will not be echoed):\n";
    bytes.strip_prefix(CRLF).or_else(|| bytes.strip_prefix(LF)).unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_rejects_prompt_injection_and_preserves_unicode() {
        for value in ["", "a\0b", "a\rb", "a\nb", "a\u{1a}b"] {
            assert!(validate(value).is_err());
        }
        assert!(validate(" spaces 中文🔐 ").is_ok());
        for command in ["a", "u", "d", "rn"] {
            assert!(needs_prompt_switch(command));
        }
        for command in ["l", "t", "x", "e", "h", "i"] {
            assert!(!needs_prompt_switch(command));
        }
    }

    #[test]
    fn only_exact_leading_prompt_is_removed() {
        let listing = b"Path = Enter password (will not be echoed):\r\nSize = 7\r\n\r\n";
        assert_eq!(listing_payload(listing), listing);
        let mut with_prompt = b"\r\nEnter password (will not be echoed):\r\n".to_vec();
        with_prompt.extend(listing);
        assert_eq!(listing_payload(&with_prompt), listing);
        let malformed = b"Path = bad\r\nEnter password (will not be echoed):\r\n";
        assert_eq!(listing_payload(malformed), malformed);
        let unknown = b"\r\nEnter password:\r\nPath = bad\r\n";
        assert_eq!(listing_payload(unknown), unknown);
    }
}
