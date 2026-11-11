//! A .zip of a few files, written by hand (`src/main/zip.ts`): deflated, or stored when that is
//! not smaller, with UTF-8 names and the given modification time.

use std::io::Write;

use chrono::{Datelike, NaiveDateTime, Timelike};
use flate2::write::DeflateEncoder;
use flate2::Compression;

pub struct ZipFile<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// MS-DOS time and date, as zip entries keep them (local time, 2-second steps).
fn dos_date_time(when: NaiveDateTime) -> (u16, u16) {
    let time = (when.hour() << 11) | (when.minute() << 5) | (when.second() >> 1);
    let date = ((when.year() as u32).saturating_sub(1980) << 9) | (when.month() << 5) | when.day();
    (time as u16, date as u16)
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// The zip of `files`, dated `when` (local time).
pub fn zip_files(files: &[ZipFile<'_>], when: NaiveDateTime) -> std::io::Result<Vec<u8>> {
    let (time, date) = dos_date_time(when);
    let mut parts: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for file in files {
        let name = file.name.as_bytes();
        let crc = crc32fast::hash(file.data);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(file.data)?;
        let deflated = encoder.finish()?;
        let stored = deflated.len() >= file.data.len();
        let body: &[u8] = if stored { file.data } else { &deflated };
        let method = if stored { 0 } else { 8 };
        let offset = parts.len() as u32;

        put32(&mut parts, 0x0403_4b50);
        put16(&mut parts, 20);
        put16(&mut parts, 0x0800); // UTF-8 names
        put16(&mut parts, method);
        put16(&mut parts, time);
        put16(&mut parts, date);
        put32(&mut parts, crc);
        put32(&mut parts, body.len() as u32);
        put32(&mut parts, file.data.len() as u32);
        put16(&mut parts, name.len() as u16);
        put16(&mut parts, 0);
        parts.extend_from_slice(name);
        parts.extend_from_slice(body);

        put32(&mut central, 0x0201_4b50);
        put16(&mut central, 20);
        put16(&mut central, 20);
        put16(&mut central, 0x0800);
        put16(&mut central, method);
        put16(&mut central, time);
        put16(&mut central, date);
        put32(&mut central, crc);
        put32(&mut central, body.len() as u32);
        put32(&mut central, file.data.len() as u32);
        put16(&mut central, name.len() as u16);
        central.extend_from_slice(&[0u8; 12]); // extra, comment, disk, internal and external attributes
        put32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let mut end = Vec::new();
    put32(&mut end, 0x0605_4b50);
    put16(&mut end, 0);
    put16(&mut end, 0);
    put16(&mut end, files.len() as u16);
    put16(&mut end, files.len() as u16);
    put32(&mut end, central.len() as u32);
    put32(&mut end, parts.len() as u32);
    put16(&mut end, 0);
    parts.extend_from_slice(&central);
    parts.extend_from_slice(&end);
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[cfg(target_os = "macos")]
    #[test]
    fn unzips_with_the_system_tools_contents_and_dates_intact() {
        let when = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(14, 30, 20).unwrap();
        let text = "xin chào ".repeat(200);
        let zip = zip_files(&[ZipFile { name: "Tên có dấu.txt", data: text.as_bytes() }, ZipFile { name: "b.bin", data: &[1, 2, 3] }], when).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.zip"), zip).unwrap();
        let status = std::process::Command::new("ditto").arg("-x").arg("-k").arg(dir.path().join("a.zip")).arg(dir.path().join("out")).status().unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(dir.path().join("out/Tên có dấu.txt")).unwrap(), text);
        assert_eq!(std::fs::read(dir.path().join("out/b.bin")).unwrap(), [1, 2, 3]);
        let modified: chrono::DateTime<chrono::Local> = std::fs::metadata(dir.path().join("out/b.bin")).unwrap().modified().unwrap().into();
        assert_eq!(modified.naive_local(), when);
    }

    #[test]
    fn stores_what_does_not_shrink_and_deflates_the_rest() {
        let when = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
        let zip = zip_files(&[ZipFile { name: "a", data: &[1, 2, 3] }, ZipFile { name: "b", data: &[7u8; 500] }], when).unwrap();
        // The method field of each local header is at offset 8.
        assert_eq!(u16::from_le_bytes([zip[8], zip[9]]), 0);
        let second = 30 + 1 + 3;
        assert_eq!(u16::from_le_bytes([zip[second + 8], zip[second + 9]]), 8);
        assert_eq!(&zip[zip.len() - 22..zip.len() - 18], [0x50, 0x4b, 5, 6]);
        assert_eq!(u16::from_le_bytes([zip[zip.len() - 12], zip[zip.len() - 11]]), 2);
    }

    #[test]
    fn dates_before_the_dos_epoch_do_not_underflow() {
        let when = NaiveDate::from_ymd_opt(1975, 6, 1).unwrap().and_hms_opt(1, 2, 3).unwrap();
        assert_eq!(dos_date_time(when).1 >> 9, 0);
    }
}
