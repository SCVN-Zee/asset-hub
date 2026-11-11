//! An uncompressed POSIX ustar archive of regular files (`src/core/export/tar.ts`), written by hand so
//! that the bytes match the TypeScript writer exactly.

use std::borrow::Cow;

const BLOCK: usize = 512;

/// One file of the archive.
#[derive(Debug, Clone)]
pub struct TarFile<'a> {
    pub name: String,
    pub data: Cow<'a, [u8]>,
}

impl<'a> TarFile<'a> {
    pub fn new(name: impl Into<String>, data: impl Into<Cow<'a, [u8]>>) -> Self {
        Self { name: name.into(), data: data.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TarError {
    /// The TypeScript throws `Tar entry name too long: <name>` for a name above 100 bytes.
    #[error("Tar entry name too long: {0}")]
    NameTooLong(String),
}

/// `buf.set(enc.encode(value).subarray(0, length), offset)`.
fn write_string(buf: &mut [u8], offset: usize, length: usize, value: &str) {
    let bytes = value.as_bytes();
    let n = bytes.len().min(length);
    buf[offset..offset + n].copy_from_slice(&bytes[..n]);
}

/// An octal number padded with zeros to `length - 1` digits and ended by a NUL. A number too long for
/// the field is cut, as the TypeScript cuts it.
fn write_octal(buf: &mut [u8], offset: usize, length: usize, value: u64) {
    let digits = format!("{value:o}");
    let padded = format!("{digits:0>width$}\0", width = length - 1);
    write_string(buf, offset, length, &padded);
}

fn header(name: &str, size: u64, mtime: u64) -> Result<[u8; BLOCK], TarError> {
    let mut h = [0u8; BLOCK];
    let bytes = name.as_bytes();
    if bytes.len() > 100 {
        return Err(TarError::NameTooLong(name.to_string()));
    }
    h[..bytes.len()].copy_from_slice(bytes);
    write_octal(&mut h, 100, 8, 0o644); // mode
    write_octal(&mut h, 108, 8, 0); // uid
    write_octal(&mut h, 116, 8, 0); // gid
    write_octal(&mut h, 124, 12, size);
    write_octal(&mut h, 136, 12, mtime);
    h[148..156].fill(0x20); // the checksum is computed with this field as spaces
    h[156] = 0x30; // typeflag '0': regular file
    write_string(&mut h, 257, 6, "ustar\0");
    write_string(&mut h, 263, 2, "00");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    write_string(&mut h, 148, 8, &format!("{sum:06o}\0 "));
    Ok(h)
}

/// Uncompressed POSIX ustar archive of regular files. `mtime` is in seconds since the epoch
/// (`Math.floor(Date.now() / 1000)` in the TypeScript default).
pub fn tar(files: &[TarFile<'_>], mtime: u64) -> Result<Vec<u8>, TarError> {
    let total = BLOCK * 2 + files.iter().map(|f| BLOCK + f.data.len().div_ceil(BLOCK) * BLOCK).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    for f in files {
        out.extend_from_slice(&header(&f.name, f.data.len() as u64, mtime)?);
        out.extend_from_slice(&f.data);
        out.resize(out.len() + (f.data.len().div_ceil(BLOCK) * BLOCK - f.data.len()), 0);
    }
    // Two zero blocks end the archive.
    out.resize(total, 0);
    Ok(out)
}

/// Seconds since the epoch now: the default `mtime` of the TypeScript writer.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_above_100_bytes_is_refused() {
        let files = [TarFile::new("x".repeat(101), vec![1u8])];
        assert_eq!(tar(&files, 0), Err(TarError::NameTooLong("x".repeat(101))));
    }

    #[test]
    fn a_name_of_100_bytes_is_accepted() {
        let files = [TarFile::new("x".repeat(100), vec![1u8, 2, 3])];
        let out = tar(&files, 0).unwrap();
        assert_eq!(out.len(), 512 + 512 + 1024);
    }

    #[test]
    fn an_empty_archive_is_two_zero_blocks() {
        assert_eq!(tar(&[], 5).unwrap(), vec![0u8; 1024]);
    }
}
