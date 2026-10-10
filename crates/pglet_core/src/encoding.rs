//! The server encoding, and the one scan that decides whether bytes are valid in it.
//!
//! **The rule: text entering the engine is validated in the server encoding where it
//! enters.** It has measured exceptions, named at the end of this paragraph.
//! 18.6 validates with the UTF-8 verifier `pg_utf8_verifystr` at the doors measured
//! here: the scanner's `pg_verifymbstr` at an `E'…'` literal's closing quote,
//! `CopyReadAttributesText`'s `pg_verifymbstr` on a de-escaped COPY field,
//! `CopyConvertBuf`'s `pg_encoding_verifymbstr` on the raw COPY input, and
//! `pg_any_to_server` behind a text-format bind parameter, `textrecv`'s
//! `pq_getmsgtext` and `pq_getmsgstring`. pglet spells it as
//! [`first_invalid_utf8_offset`], reached by the literal decoder, both COPY routes, a
//! bound `$N` on the native server in either wire format (the binary jsonb payload
//! among them, scanned behind the version byte its first byte always is —
//! `docs/param_binding.md`), a parameter the wasm host binds from JS, and the dump's
//! COPY payload.
//! The doors measured not to reach it — the frontend message strings `pq_getmsgstring`
//! validates on 18.6 (`EDGES.md` row 130), SQL text handed to an in-process door
//! (row 133), and a `Value` bound through `Database::execute_bound` — are
//! `docs/encoding.md`'s residuals.
//!
//! **A bare `String::from_utf8` is not that check.** Rust's UTF-8 acceptance set is
//! `pg_utf8_islegal`'s byte for byte — overlongs, surrogates and values above
//! `U+10FFFF` are refused by both — but Rust **accepts** a NUL and
//! `pg_utf8_verifystr` stops at one (`if (*s == '\0') break;`, the same guard on the
//! byte-wise `pg_utf8_verifychar`). So the scan below looks for the NUL separately and
//! takes whichever failure comes first, left to right.
//!
//! The failure *offset* and the reported *bytes* are two facts, not one: 18.6 reports
//! `min(mblen, remaining)` bytes from the offset, and `remaining` is the length of the
//! buffer it was handed — which for COPY's raw route is the whole unverified input and
//! not the line the failure sits in. So [`invalid_encoding_bytes`] takes the buffer to
//! report from as its own argument, and a caller that verified a narrower slice passes
//! the wider one.

use crate::error::{Error, Result};

/// The offset of the first byte that is not valid in the server encoding, scanning
/// left to right — the first NUL or the first byte that starts no legal UTF-8
/// sequence, whichever comes first.
///
/// This is the one scan; `pg_utf8_verifystr` returns the count of valid bytes and
/// `pg_verify_mbstr` turns `oklen != len` into the error, which is the same fact.
#[must_use]
pub fn first_invalid_utf8_offset(bytes: &[u8]) -> Option<usize> {
    let nul = bytes.iter().position(|&b| b == 0);
    match std::str::from_utf8(bytes) {
        Ok(_) => nul,
        Err(e) => {
            let invalid = e.valid_up_to();
            Some(nul.map_or(invalid, |n| n.min(invalid)))
        }
    }
}

/// `report_invalid_encoding`: the bytes from `offset`, `min(pg_utf_mblen(lead),
/// remaining)` of them.
///
/// `all` is the buffer 18.6 would have been verifying, which is not always the slice
/// the caller scanned — see the module doc.
#[must_use]
pub fn invalid_encoding_bytes(all: &[u8], offset: usize) -> Vec<u8> {
    let rest = all.get(offset..).unwrap_or_default();
    let mblen = rest.first().map_or(0, |&lead| utf8_lead_len(lead));
    // `Min(mblen, remaining)` is the `get`: it yields `None` exactly when the lead byte
    // asks for more bytes than are left, and the fallback is what remains.
    rest.get(..mblen).unwrap_or(rest).to_vec()
}

/// The bytes [`Error::InvalidEncoding`] would name for `bytes`, or `None` when they
/// are valid in the server encoding.
///
/// The two halves above composed over one buffer — what a caller wants when the slice
/// it verified is also the slice 18.6 reports from.
#[must_use]
pub fn utf8_failure_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    first_invalid_utf8_offset(bytes).map(|offset| invalid_encoding_bytes(bytes, offset))
}

/// Validate `bytes` in the server encoding and take them as a `String`.
///
/// # Errors
///
/// [`Error::InvalidEncoding`] (`22021`) naming the bytes from the first failure.
pub fn text_from_utf8(bytes: Vec<u8>) -> Result<String> {
    if let Some(report) = utf8_failure_bytes(&bytes) {
        return Err(Error::InvalidEncoding { bytes: report });
    }
    // The scan above refuses a strict superset of what `from_utf8` refuses, so this
    // cannot fail; `map_err` rather than an `expect` keeps the library-code no-panic rule.
    String::from_utf8(bytes).map_err(|e| Error::InvalidEncoding {
        bytes: e.into_bytes(),
    })
}

/// `pg_utf_mblen`: a UTF-8 **lead-byte** length. It answers 1, 2, 3 or 4 and never
/// more — the 5- and 6-byte arms are `#ifdef NOT_USED` — which is why 18.6's further
/// `Min(jlimit, 8)` cap on the reported byte count has no counterpart here.
const fn utf8_lead_len(lead: u8) -> usize {
    match lead {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nul_is_a_failure_where_from_utf8_accepts_it() {
        // The whole reason this module exists: `String::from_utf8` takes a NUL.
        assert!(String::from_utf8(b"x\0y".to_vec()).is_ok());
        assert_eq!(first_invalid_utf8_offset(b"x\0y"), Some(1));
        assert_eq!(utf8_failure_bytes(b"x\0y"), Some(vec![0x00]));
    }

    #[test]
    fn the_first_failure_wins_whichever_kind_it_is() {
        // A NUL before an illegal byte and an illegal byte before a NUL.
        assert_eq!(first_invalid_utf8_offset(b"a\0b\xe9c"), Some(1));
        assert_eq!(first_invalid_utf8_offset(b"a\xe9b\0c"), Some(1));
        assert_eq!(utf8_failure_bytes(b"a\0b\xe9c"), Some(vec![0x00]));
        assert_eq!(
            utf8_failure_bytes(b"a\xe9b\0c"),
            Some(vec![0xe9, 0x62, 0x00])
        );
    }

    #[test]
    fn a_valid_multibyte_character_passes() {
        assert_eq!(first_invalid_utf8_offset("é".as_bytes()), None);
        assert_eq!(first_invalid_utf8_offset("💩".as_bytes()), None);
        assert_eq!(text_from_utf8("xéy".as_bytes().to_vec()).unwrap(), "xéy");
    }

    #[test]
    fn the_report_is_clamped_by_what_remains_in_the_buffer_it_is_given() {
        // `min(pg_utf_mblen(0xe9) = 3, remaining)`, and `remaining` is the argument's.
        assert_eq!(utf8_failure_bytes(b"x\xe9"), Some(vec![0xe9]));
        assert_eq!(utf8_failure_bytes(b"x\xe9y"), Some(vec![0xe9, 0x79]));
        assert_eq!(
            utf8_failure_bytes(b"x\xe9y\n"),
            Some(vec![0xe9, 0x79, 0x0a])
        );
        // A caller that scanned a narrower slice reports from the wider one: the
        // COPY raw route verifies a line and 18.6 verifies the whole input, so the
        // trailing newline is in the byte list. Measured on 18.6: `COPY` of
        // `x\xe9y\n` answers `0xe9 0x79 0x0a`.
        let all = b"x\xe9y\ny\n";
        let line = &all[..4];
        let offset = first_invalid_utf8_offset(line).unwrap();
        assert_eq!(invalid_encoding_bytes(all, offset), vec![0xe9, 0x79, 0x0a]);
    }

    #[test]
    fn the_error_renders_18_6s_sentence() {
        let err = text_from_utf8(b"x\0y".to_vec()).unwrap_err();
        assert_eq!(err.sqlstate(), "22021");
        assert_eq!(
            err.to_string(),
            "invalid byte sequence for encoding \"UTF8\": 0x00"
        );
    }
}
