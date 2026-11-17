//! The binary pack of named blobs the thumbnail routes use: repeated
//! `[u16 nameLength][name utf8][u32 byteLength][bytes]`, big-endian lengths. The Worker's codec is
//! `backend/src/pack.ts`; both files hold the same test vectors. Shared by `crates/app` and the Worker.
//! The shard mapping of thumbnail names lives here too, so the client can group a request by the
//! one shard database it touches.

/// Thumbnail shard databases. The first hex digit of a thumbnail name picks one.
pub const SHARDS: usize = 8;

/// The shard of a thumbnail name, `.fx` suffix included: its first hex digit modulo 8. A name that
/// does not start with a hex digit has no shard.
pub fn shard_of(name: &str) -> Option<usize> {
    name.chars().next().and_then(|c| c.to_digit(16)).map(|digit| digit as usize % SHARDS)
}

/// A pack of `entries`, each a name and its bytes. A name longer than 65535 bytes, or data longer
/// than `u32::MAX`, cannot be written and is an error.
pub fn encode_pack(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(entries.iter().map(|(n, b)| 6 + n.len() + b.len()).sum());
    for (name, bytes) in entries {
        let name_len = u16::try_from(name.len()).map_err(|_| "pack name too long".to_owned())?;
        let byte_len = u32::try_from(bytes.len()).map_err(|_| "pack entry too large".to_owned())?;
        out.extend_from_slice(&name_len.to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&byte_len.to_be_bytes());
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// The bytes that precede an entry's data: `[u16 nameLength][name utf8][u32 byteLength]`. A name
/// longer than 65535 bytes cannot be written and is an error.
pub fn encode_entry_header(name: &str, byte_len: u32) -> Result<Vec<u8>, String> {
    let name_len = u16::try_from(name.len()).map_err(|_| "pack name too long".to_owned())?;
    let mut out = Vec::with_capacity(6 + name.len());
    out.extend_from_slice(&name_len.to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&byte_len.to_be_bytes());
    Ok(out)
}

/// The entries of `data`, borrowed from it: no name or payload is copied. A truncated pack or a
/// name that is not UTF-8 is an error.
pub fn decode_pack_ref(data: &[u8]) -> Result<Vec<(&str, &[u8])>, String> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at < data.len() {
        let name_len = u16::from_be_bytes(take(data, &mut at, 2)?.try_into().map_err(|_| TRUNCATED)?) as usize;
        let name = std::str::from_utf8(take(data, &mut at, name_len)?).map_err(|_| "invalid pack name".to_owned())?;
        let byte_len = u32::from_be_bytes(take(data, &mut at, 4)?.try_into().map_err(|_| TRUNCATED)?) as usize;
        entries.push((name, take(data, &mut at, byte_len)?));
    }
    Ok(entries)
}

/// The entries of `data`, copied. A truncated pack or a name that is not UTF-8 is an error.
pub fn decode_pack(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    Ok(decode_pack_ref(data)?.into_iter().map(|(name, bytes)| (name.to_owned(), bytes.to_vec())).collect())
}

const TRUNCATED: &str = "truncated pack";

/// The next `len` bytes of `data` from `at`, which moves past them.
fn take<'a>(data: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = at.checked_add(len).filter(|end| *end <= data.len()).ok_or(TRUNCATED)?;
    let slice = &data[*at..end];
    *at = end;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_map_to_known_shards() {
        let name = |digit: char| format!("{digit}{}", "0".repeat(39));
        let pinned = [('0', 0), ('7', 7), ('8', 0), ('9', 1), ('a', 2), ('c', 4), ('f', 7)];
        for (digit, shard) in pinned {
            assert_eq!(shard_of(&name(digit)), Some(shard), "{digit}");
            assert_eq!(shard_of(&format!("{}.fx", name(digit))), Some(shard), "{digit}.fx");
        }
        assert_eq!(shard_of(""), None);
        assert_eq!(shard_of("zz"), None);
        assert_eq!(shard_of(".fx"), None);
    }

    type Entry = (String, Vec<u8>);

    fn from_hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    /// The vectors of `backend/src/pack.test.ts`.
    fn vectors() -> Vec<(Vec<Entry>, Vec<u8>)> {
        vec![
            (vec![("ab".into(), vec![1, 2, 3])], from_hex(&format!("{}{}{}", "00026162", "00000003", "010203"))),
            (vec![("x.fx".into(), vec![]), ("é".into(), vec![255])], from_hex(&format!("{}{}{}{}{}", "0004782e6678", "00000000", "0002c3a9", "00000001", "ff"))),
        ]
    }

    #[test]
    fn encodes_vectors() {
        for (entries, bytes) in vectors() {
            assert_eq!(encode_pack(&entries).unwrap(), bytes);
        }
        assert!(encode_pack(&[("n".repeat(65536), vec![])]).is_err());
    }

    #[test]
    fn decodes_vectors() {
        for (entries, bytes) in vectors() {
            assert_eq!(decode_pack(&bytes).unwrap(), entries);
        }
        assert_eq!(decode_pack(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn borrowed_decode_matches_the_copying_decode() {
        for (entries, bytes) in vectors() {
            let borrowed: Vec<Entry> = decode_pack_ref(&bytes).unwrap().into_iter().map(|(n, b)| (n.to_owned(), b.to_vec())).collect();
            assert_eq!(borrowed, entries);
        }
        assert!(decode_pack_ref(&from_hex("0001ff00000000")).is_err());
        assert!(decode_pack_ref(&from_hex("0001ff00ffffffff")).is_err());
    }

    #[test]
    fn truncated_is_error() {
        for (_, bytes) in vectors() {
            for cut in 1..bytes.len() {
                // A cut right after an entry is a shorter, valid pack.
                if let Ok(entries) = decode_pack(&bytes[..cut]) {
                    assert_eq!(encode_pack(&entries).unwrap().len(), cut);
                }
            }
        }
        let first = &vectors()[0].1;
        for cut in 1..first.len() {
            assert!(decode_pack(&first[..cut]).is_err(), "cut at {cut}");
        }
        // A length that points far past the end must not overflow.
        assert!(decode_pack(&from_hex("0001ff00ffffffff")).is_err());
    }

    #[test]
    fn header_is_the_start_of_an_entry() {
        let header = encode_entry_header("x.fx", 3).unwrap();
        let entry = encode_pack(&[("x.fx".into(), vec![1, 2, 3])]).unwrap();
        assert_eq!(header, entry[..header.len()]);
        assert_eq!(header, from_hex("0004782e667800000003"));
        assert!(encode_entry_header(&"n".repeat(65536), 0).is_err());
    }

    #[test]
    fn bad_utf8_is_error() {
        assert!(decode_pack(&from_hex("0001ff00000000")).is_err());
    }
}
