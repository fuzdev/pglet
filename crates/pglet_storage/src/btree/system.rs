//! Codecs for the `0x02` system entries, the free-list chain and the retired-page log.
//!
//! Pure encode/decode helpers — no I/O, no behavior — for what the store keeps
//! outside the fixed meta slot:
//!
//! - **Catalog chunking** — the postcard `Schema` blob is split into
//!   [`CATALOG_CHUNK_SIZE`] slices, each stored under a `SYS_CATALOG` key;
//!   reassembled on open by concatenating the chunk values in chunk-index order.
//! - **Table meta** — `(name, next_row_id)` ↔ bytes, the value of a `SYS_TABLE`
//!   entry (a table's directory entry and its row-id counter in one).
//! - **Free-list page body** — `[next_page][next_crc][count][PageId; count]`, one
//!   link of the free-list chain. Kept **body-only** (the page's own checksum is
//!   `double_meta.rs`'s to stamp); the chain head and its checksum live in the meta
//!   slot, and each body carries the checksum of the page after it.
//! - **Retired-page log** — a chain page body
//!   `[next_page][next_crc][next_max][count][(PageId, freed_version); count]`, and the
//!   `[count][(PageId, freed_version); count]` entries a meta slot carries after its fixed
//!   metadata.

use super::page::{DATA_SIZE, PageId};

/// Max bytes of catalog blob per `SYS_CATALOG` chunk value.
///
/// A chunk is an ordinary leaf value: one this long is stored out of line, in an
/// overflow chain, like any other leaf value over the inline limit
/// (`page::INLINE_VALUE_MAX`). The chunking bounds what one catalog rewrite hands
/// one entry; it is not what makes the catalog fit a page.
pub(super) const CATALOG_CHUNK_SIZE: usize = 3072;

/// Split a catalog blob into ordered, page-sized chunks.
///
/// An empty blob yields **no** chunks (an empty catalog persists zero
/// `SYS_CATALOG` entries). Reassembly is plain concatenation of the chunk
/// values in chunk-index order.
pub(super) fn chunk_catalog(blob: &[u8]) -> Vec<Vec<u8>> {
    blob.chunks(CATALOG_CHUNK_SIZE)
        .map(<[u8]>::to_vec)
        .collect()
}

/// Encode a `SYS_TABLE` value: `[name_len: u32 LE][name][next_row_id: u64 LE]`.
pub(super) fn encode_sys_table(name: &str, next_row_id: u64) -> Vec<u8> {
    let name = name.as_bytes();
    let mut out = Vec::with_capacity(4 + name.len() + 8);
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&next_row_id.to_le_bytes());
    out
}

/// Decode a `SYS_TABLE` value into `(name, next_row_id)`, or `None` if the
/// bytes are truncated or the name is not valid UTF-8.
pub(super) fn decode_sys_table(bytes: &[u8]) -> Option<(String, u64)> {
    let name_len = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    let name_end = 4usize.checked_add(name_len)?;
    let name = String::from_utf8(bytes.get(4..name_end)?.to_vec()).ok()?;
    let row_end = name_end.checked_add(8)?;
    let next_row_id = u64::from_le_bytes(bytes.get(name_end..row_end)?.try_into().ok()?);
    Some((name, next_row_id))
}

/// Bytes of fixed header in a free-list page body: `next_page(8) + next_crc(4) +
/// count(4)`.
const FREELIST_HEADER: usize = 8 + 4 + 4;

/// Number of `PageId`s one free-list chain page can hold.
///
/// Sized against [`DATA_SIZE`] (the usable body of a page once a 32-byte page
/// header is reserved) — a conservative bound, since `double_meta.rs` writes the body at
/// the start of a full page, before a 4-byte checksum tail.
pub(super) const FREELIST_IDS_PER_PAGE: usize = (DATA_SIZE - FREELIST_HEADER) / 8;

/// Encode one free-list chain page **body**: `[next_page: u64 LE][next_crc: u32
/// LE][count: u32 LE][PageId (u64 LE); count]`. `next_page` is the next link
/// ([`NULL_PAGE_ID`](super::page::NULL_PAGE_ID) at the tail) and `next_crc` the
/// checksum that link's page carries (0 at the tail), so each page vouches for its
/// successor as the slot vouches for the head. Only the first
/// [`FREELIST_IDS_PER_PAGE`] ids are written; the caller chains the rest.
pub(super) fn encode_freelist_body(next_page: PageId, next_crc: u32, ids: &[PageId]) -> Vec<u8> {
    let count = ids.len().min(FREELIST_IDS_PER_PAGE);
    let mut out = Vec::with_capacity(FREELIST_HEADER + count * 8);
    out.extend_from_slice(&next_page.to_le_bytes());
    out.extend_from_slice(&next_crc.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for &id in ids.iter().take(count) {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

/// A decoded free-list chain page body: the next link, the checksum that link's page
/// must carry, and the ids this page lists.
pub(super) type FreelistBody = (PageId, u32, Vec<PageId>);

/// Decode a free-list chain page body, or `None` if the buffer is too short or the
/// count overflows a page (a torn/garbage link — the caller treats that as the end of
/// a recoverable chain).
pub(super) fn decode_freelist_body(buf: &[u8]) -> Option<FreelistBody> {
    let next_page = PageId::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
    let next_crc = u32::from_le_bytes(buf.get(8..12)?.try_into().ok()?);
    let count = u32::from_le_bytes(buf.get(12..16)?.try_into().ok()?) as usize;
    if count > FREELIST_IDS_PER_PAGE {
        return None;
    }
    let mut ids = Vec::with_capacity(count);
    let mut off = FREELIST_HEADER;
    for _ in 0..count {
        let end = off.checked_add(8)?;
        ids.push(PageId::from_le_bytes(buf.get(off..end)?.try_into().ok()?));
        off = end;
    }
    Some((next_page, next_crc, ids))
}

/// A retired-page log entry: a page superseded and not yet freed, and its
/// `freed_version` — the last tree version that holds it.
pub(super) type RetiredEntry = (PageId, u64);

/// `[next_page: u64][next_crc: u32][next_max: u64][count: u32]` — a retired chain
/// page's header.
const RETIRED_HEADER: usize = 8 + 4 + 8 + 4;

/// Number of [`RetiredEntry`]s one retired chain page can hold, sized against
/// [`DATA_SIZE`] like [`FREELIST_IDS_PER_PAGE`].
pub(super) const RETIRED_PER_PAGE: usize = (DATA_SIZE - RETIRED_HEADER) / 16;

/// Encode one retired chain page **body**: `[next_page: u64 LE][next_crc: u32
/// LE][next_max: u64 LE][count: u32 LE][(PageId: u64 LE, freed_version: u64 LE);
/// count]`. The link carries its page's checksum, as the free-list chain's does, and
/// the largest `freed_version` it lists (`next_max`), so a walk can stop before
/// reading a page every entry of which is below its floor. Only the first
/// [`RETIRED_PER_PAGE`] entries are written; the caller chains the rest.
pub(super) fn encode_retired_body(
    next_page: PageId,
    next_crc: u32,
    next_max: u64,
    entries: &[RetiredEntry],
) -> Vec<u8> {
    let count = entries.len().min(RETIRED_PER_PAGE);
    let mut out = Vec::with_capacity(RETIRED_HEADER + count * 16);
    out.extend_from_slice(&next_page.to_le_bytes());
    out.extend_from_slice(&next_crc.to_le_bytes());
    out.extend_from_slice(&next_max.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for &(id, freed_version) in entries.iter().take(count) {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&freed_version.to_le_bytes());
    }
    out
}

/// A decoded retired chain page body: the next link, the checksum and largest
/// `freed_version` of that link's page, and this page's entries.
pub(super) type RetiredBody = (PageId, u32, u64, Vec<RetiredEntry>);

/// Decode a retired chain page body, or `None` if the buffer is too short or the
/// count overflows a page.
pub(super) fn decode_retired_body(buf: &[u8]) -> Option<RetiredBody> {
    let next_page = PageId::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
    let next_crc = u32::from_le_bytes(buf.get(8..12)?.try_into().ok()?);
    let next_max = u64::from_le_bytes(buf.get(12..20)?.try_into().ok()?);
    let count = u32::from_le_bytes(buf.get(20..24)?.try_into().ok()?) as usize;
    if count > RETIRED_PER_PAGE {
        return None;
    }
    let entries = decode_entries(buf.get(RETIRED_HEADER..)?, count)?;
    Some((next_page, next_crc, next_max, entries))
}

/// Encode the retired entries a meta slot carries after its fixed metadata:
/// `[count: u32 LE][(PageId: u64 LE, freed_version: u64 LE); count]`.
pub(super) fn encode_retired_entries(entries: &[RetiredEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + entries.len() * 16);
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for &(id, freed_version) in entries {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&freed_version.to_le_bytes());
    }
    out
}

/// Decode a meta slot's retired entries, or `None` if the bytes are short of the
/// count they claim or run past it — every byte of the slot payload is accounted for.
pub(super) fn decode_retired_entries(buf: &[u8]) -> Option<Vec<RetiredEntry>> {
    let count = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?) as usize;
    if buf.len() != 4 + count.checked_mul(16)? {
        return None;
    }
    decode_entries(&buf[4..], count)
}

fn decode_entries(buf: &[u8], count: usize) -> Option<Vec<RetiredEntry>> {
    let mut entries = Vec::with_capacity(count.min(buf.len() / 16));
    for i in 0..count {
        let at = i.checked_mul(16)?;
        let id = PageId::from_le_bytes(buf.get(at..at + 8)?.try_into().ok()?);
        let freed_version = u64::from_le_bytes(buf.get(at + 8..at + 16)?.try_into().ok()?);
        entries.push((id, freed_version));
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::super::page::NULL_PAGE_ID;
    use super::*;

    #[test]
    fn sys_table_roundtrips() {
        for (name, next) in [
            ("users", 0u64),
            ("a_very_long_table_name", 1 << 40),
            ("", 7),
        ] {
            let bytes = encode_sys_table(name, next);
            assert_eq!(decode_sys_table(&bytes), Some((name.to_string(), next)));
        }
    }

    #[test]
    fn sys_table_decode_rejects_truncation() {
        let bytes = encode_sys_table("users", 42);
        for cut in 0..bytes.len() {
            assert!(
                decode_sys_table(&bytes[..cut]).is_none(),
                "truncated to {cut} bytes must not decode"
            );
        }
        // Full buffer decodes.
        assert!(decode_sys_table(&bytes).is_some());
    }

    #[test]
    fn catalog_chunks_reassemble() {
        // Empty blob → zero chunks.
        assert!(chunk_catalog(&[]).is_empty());

        for len in [
            1usize,
            CATALOG_CHUNK_SIZE - 1,
            CATALOG_CHUNK_SIZE,
            CATALOG_CHUNK_SIZE * 3 + 5,
        ] {
            let blob: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let chunks = chunk_catalog(&blob);
            // Every chunk fits the cap; reassembly is concatenation.
            assert!(chunks.iter().all(|c| c.len() <= CATALOG_CHUNK_SIZE));
            let reassembled: Vec<u8> = chunks.into_iter().flatten().collect();
            assert_eq!(reassembled, blob, "len {len} must round-trip");
        }
    }

    #[test]
    fn freelist_body_roundtrips() {
        for ids in [vec![], vec![1u64], vec![5, 9, NULL_PAGE_ID, 0, 42]] {
            let body = encode_freelist_body(NULL_PAGE_ID, 0, &ids);
            assert_eq!(
                decode_freelist_body(&body),
                Some((NULL_PAGE_ID, 0, ids.clone()))
            );
            // A non-null next link and its checksum round-trip too.
            let body2 = encode_freelist_body(7, 0xFEED_F00D, &ids);
            assert_eq!(decode_freelist_body(&body2), Some((7, 0xFEED_F00D, ids)));
        }
    }

    #[test]
    fn freelist_body_caps_ids_per_page() {
        // More ids than fit are truncated to one page's worth (the caller chains
        // the remainder); the encoded body decodes back to exactly that prefix.
        let ids: Vec<u64> = (0..(FREELIST_IDS_PER_PAGE as u64 + 100)).collect();
        let body = encode_freelist_body(NULL_PAGE_ID, 0, &ids);
        let (next, _, decoded) = decode_freelist_body(&body).unwrap();
        assert_eq!(next, NULL_PAGE_ID);
        assert_eq!(decoded, ids[..FREELIST_IDS_PER_PAGE]);
    }

    #[test]
    fn retired_body_and_entries_roundtrip() {
        let entries: Vec<RetiredEntry> = (0..(RETIRED_PER_PAGE as u64 + 3))
            .map(|i| (i * 3, 100 - i % 7))
            .collect();
        let body = encode_retired_body(9, 0xABCD, 77, &entries);
        let (next, crc, max, decoded) = decode_retired_body(&body).unwrap();
        assert_eq!((next, crc, max), (9, 0xABCD, 77));
        assert_eq!(decoded, entries[..RETIRED_PER_PAGE]);
        let inline = encode_retired_entries(&entries[..5]);
        assert_eq!(decode_retired_entries(&inline).unwrap(), entries[..5]);
        assert_eq!(
            decode_retired_entries(&encode_retired_entries(&[])).unwrap(),
            []
        );
        // a count the bytes do not match, either way, is refused
        assert!(decode_retired_entries(&inline[..inline.len() - 1]).is_none());
        let mut long = inline;
        long.push(0);
        assert!(decode_retired_entries(&long).is_none());
        let mut body = body;
        body[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_retired_body(&body).is_none());
    }

    #[test]
    fn freelist_body_decode_rejects_bad_count() {
        // count claims more ids than the buffer holds → None (torn link).
        let mut body = encode_freelist_body(NULL_PAGE_ID, 0, &[1, 2, 3]);
        body[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_freelist_body(&body).is_none());
    }
}
