//! Hex transport of thumbnail bytes. D1 turns a bound typed array into a JSON number array and
//! returns every BLOB as one, and each element costs CPU time. A hex string crosses the binding as
//! one value instead. SQL turns it into a BLOB (`unhex`) or back (`hex`), so the stored bytes do
//! not change.

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Value of each ASCII hex digit of either case, and `INVALID` for every other byte.
const INVALID: u8 = 0xFF;
const VALUES: [u8; 256] = {
    let mut table = [INVALID; 256];
    let mut i = 0;
    while i < 16 {
        table[DIGITS[i] as usize] = i as u8;
        if i >= 10 {
            table[(DIGITS[i] - 32) as usize] = i as u8;
        }
        i += 1;
    }
    table
};

/// Two lower-case hex digits for each byte.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)]);
        out.push(DIGITS[usize::from(byte & 0x0F)]);
    }
    // Every pushed byte is an ASCII digit, so the conversion cannot fail.
    String::from_utf8(out).unwrap_or_default()
}

/// Appends the bytes of `hex` to `out`. `None` for an odd length or a character that is not a hex
/// digit, and then `out` is unchanged.
pub fn decode_into(hex: &str, out: &mut Vec<u8>) -> Option<()> {
    let digits = hex.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let start = out.len();
    out.reserve(digits.len() / 2);
    let (pairs, _) = digits.as_chunks::<2>();
    for &[first, second] in pairs {
        let high = VALUES[usize::from(first)];
        let low = VALUES[usize::from(second)];
        if high == INVALID || low == INVALID {
            out.truncate(start);
            return None;
        }
        out.push(high << 4 | low);
    }
    Some(())
}

/// The bytes of `hex`, or `None` as `decode_into` does.
#[cfg(test)]
pub fn decode(hex: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    decode_into(hex, &mut out).map(|()| out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_value_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        let hex = encode(&all);
        assert_eq!(hex.len(), 512);
        assert_eq!(decode(&hex), Some(all));
    }

    #[test]
    fn encoding_is_lower_case_and_matches_known_values() {
        assert_eq!(encode(&[]), "");
        assert_eq!(encode(&[0x00, 0xFF, 0xAB, 0x0F]), "00ffab0f");
    }

    #[test]
    fn decoding_accepts_both_cases() {
        assert_eq!(decode("00FFabCD"), Some(vec![0x00, 0xFF, 0xAB, 0xCD]));
        assert_eq!(decode(""), Some(Vec::new()));
    }

    #[test]
    fn decoding_rejects_odd_length_and_bad_characters() {
        for bad in ["a", "abc", "0g", "zz", "0x", "a ", "é1", "-1", "+1", "1\u{0}"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_failed_decode_leaves_the_buffer_unchanged() {
        let mut out = vec![1, 2];
        assert_eq!(decode_into("0102zz", &mut out), None);
        assert_eq!(out, vec![1, 2]);
        assert_eq!(decode_into("0304", &mut out), Some(()));
        assert_eq!(out, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_thumbnail_sized_payload_stays_under_the_d1_string_limit() {
        let payload = vec![0xA5u8; 256 * 1024];
        let hex = encode(&payload);
        assert_eq!(hex.len(), 512 * 1024);
        assert!(hex.len() < 2 * 1024 * 1024);
        assert_eq!(decode(&hex), Some(payload));
    }
}
