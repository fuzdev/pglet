//! Postgres binary-format value codec.
//!
//! The binary twin of `text.rs`: `value_to_binary` encodes a [`Value`] to the
//! type-specific big-endian wire bytes real Postgres clients decode, and
//! `value_from_binary` is its inverse (used to decode binary-format Bind
//! parameters). One-dimensional arrays go through `encode_array_binary` /
//! `decode_binary_array`.

use super::json::parse_jsonb;
use super::{ArrayValue, TypeOid, Value, type_oid_display_name};
use crate::error::{Error, Result};

/// Convert a Value to wire format bytes (binary format).
///
/// Postgres binary format is type-specific big-endian encoding. The binary twin
/// of `value_to_text`; `value_from_binary` is the inverse. Returns `None` for
/// SQL NULL.
#[must_use]
pub fn value_to_binary(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(vec![u8::from(*b)]),
        Value::Int2(n) => Some(n.to_be_bytes().to_vec()),
        Value::Int4(n) => Some(n.to_be_bytes().to_vec()),
        Value::Int8(n) => Some(n.to_be_bytes().to_vec()),
        Value::Float4(n) => Some(n.to_be_bytes().to_vec()),
        Value::Float8(n) => Some(n.to_be_bytes().to_vec()),
        Value::Text(s) => Some(s.as_bytes().to_vec()),
        Value::Bytea(b) => Some(b.clone()),
        // Postgres binary format: microseconds since 2000-01-01 00:00:00 UTC, and days
        // since 2000-01-01, each with its infinity sentinels.
        Value::Timestamp(ts) => Some(ts.to_pg_micros().to_be_bytes().to_vec()),
        Value::Date(d) => Some(d.to_pg_days().to_be_bytes().to_vec()),
        Value::Time(us) => {
            // Postgres `time_send`: a bare int64 of microseconds since midnight.
            Some(us.to_be_bytes().to_vec())
        }
        // Postgres `numeric_send`: an int16 header (ndigits, weight, sign, dscale)
        // followed by the base-10000 digits (see `PgNumeric::to_wire_bytes`).
        Value::Numeric(nn) => Some(nn.to_wire_bytes()),
        Value::Jsonb(j) => {
            // JSONB binary format: version byte (1) + JSON UTF-8 bytes
            let json_bytes = j.to_json_string().into_bytes();
            let mut buf = Vec::with_capacity(1 + json_bytes.len());
            buf.push(1); // version byte
            buf.extend(json_bytes);
            Some(buf)
        }
        Value::Uuid(u) => Some(u.to_vec()),
        Value::Array(arr) => Some(encode_array_binary(arr)),
        // Postgres `interval_send`: time (i64 µs) → day (i32) → month (i32), 16 bytes.
        Value::Interval(iv) => Some(iv.to_wire_bytes().to_vec()),
    }
}

/// [`value_to_binary`] for a value in a result column declared `oid`.
///
/// An array's header names the **column's** element type where that type's binary
/// form is the one its elements are sent in, and the element type the value records
/// otherwise. `ARRAY[timestamptz '…']` builds its elements as `Value::Timestamp`, which
/// `timestamp` shares, so the value records `timestamp` (`1114`) where 18.6's header,
/// read from the column, says `timestamptz` (`1184`) over the same bytes; and an
/// all-`NULL` `ARRAY[NULL::int4]` records `text` where the column says `int4`.
///
/// The column's type is not trusted further than its elements: a declared column type
/// can itself be wrong (`ARRAY[COALESCE(i + 0, b + 0)]` over an `int4` `i` holding
/// `NULL` and an `int8` `b` is typed `int4[]`, where 18.6 says `int8[]`), and naming it
/// over elements it does not describe would pair an `int4` header with 8-byte `int8`
/// elements. So the column's element type names the
/// header only when every element that is not `NULL` is sent as that type
/// ([`value_sends_as`]); otherwise the header is the value's own record, as
/// [`value_to_binary`] writes it. Where the wrong column type shares its elements'
/// representation the elements cannot contradict it, and the header follows it: a `text`
/// array under a column typed `name[]` is headed `name` (`EDGES.md` row 36). The binary
/// twin of `value_to_text_with_oid`.
#[must_use]
pub fn value_to_binary_with_oid(v: &Value, oid: TypeOid) -> Option<Vec<u8>> {
    match (v, oid.array_element()) {
        (Value::Array(arr), Some(element))
            if arr.elements.iter().all(|e| value_sends_as(e, element)) =>
        {
            Some(encode_array_binary_as(arr, element))
        }
        _ => value_to_binary(v),
    }
}

/// Whether `oid` is a type pglet holds as `v`'s variant, so that type's binary form of `v`
/// is [`value_to_binary`]'s bytes for it.
///
/// That is narrower than "the bytes are a valid binary form of `oid`": a `Value::Text`'s
/// bytes are also a valid `bytea`, and this answers `false` there, because a `bytea` reader
/// reads them back as a `Value::Bytea` rather than as `v`. The relation is read off
/// [`value_to_binary`]'s own dispatch, which is keyed on the
/// `Value` variant: every type whose values pglet holds as that variant is sent through
/// that one arm. So `text` bytes are `varchar`'s, `name`'s, `"char"`'s and `void`'s (all
/// `Value::Text`), `int4` bytes are `oid`'s, and `timestamp` bytes are `timestamptz`'s;
/// every other scalar variant is sent as its own type only. `NULL` sends no bytes and so
/// contradicts no type. The pairings `value_from_binary` has an arm for are checked
/// against it by a round-trip test; `name`, `oid`, `"char"` and `void` have no receive arm
/// and rest on their shape-free representation (`TypeOid::Name`).
#[must_use]
pub fn value_sends_as(v: &Value, oid: TypeOid) -> bool {
    match v {
        Value::Null => true,
        Value::Text(_) => matches!(
            oid,
            TypeOid::Text | TypeOid::Varchar | TypeOid::Name | TypeOid::Char | TypeOid::Void
        ),
        Value::Int4(_) => matches!(oid, TypeOid::Int4 | TypeOid::Oid),
        Value::Timestamp(_) => matches!(oid, TypeOid::Timestamp | TypeOid::Timestamptz),
        Value::Array(arr) => oid
            .array_element()
            .is_some_and(|element| arr.elements.iter().all(|e| value_sends_as(e, element))),
        Value::Bool(_)
        | Value::Int2(_)
        | Value::Int8(_)
        | Value::Float4(_)
        | Value::Float8(_)
        | Value::Bytea(_)
        | Value::Date(_)
        | Value::Time(_)
        | Value::Numeric(_)
        | Value::Jsonb(_)
        | Value::Uuid(_)
        | Value::Interval(_) => v.type_oid() == Some(oid),
    }
}

/// Encode an array in Postgres binary array format.
///
/// Layout (big-endian): ndim, has-nulls flag, element OID, then per dimension a
/// (length, lower-bound=1) pair, then each element as a length-prefixed binary
/// value (-1 length for NULL). One-dimensional only; an empty array uses ndim 0.
/// `decode_binary_array` is the inverse.
fn encode_array_binary(arr: &ArrayValue) -> Vec<u8> {
    encode_array_binary_as(arr, arr.element_type)
}

/// [`encode_array_binary`] with the header's element OID `element`.
fn encode_array_binary_as(arr: &ArrayValue, element: TypeOid) -> Vec<u8> {
    let elem_oid = element as i32;
    let has_nulls = arr.elements.iter().any(Value::is_null);
    let ndim: i32 = i32::from(!arr.elements.is_empty());

    let mut buf = Vec::new();
    buf.extend_from_slice(&ndim.to_be_bytes());
    buf.extend_from_slice(&i32::from(has_nulls).to_be_bytes());
    buf.extend_from_slice(&elem_oid.to_be_bytes());

    if ndim == 1 {
        let len = i32::try_from(arr.elements.len()).unwrap_or(i32::MAX);
        buf.extend_from_slice(&len.to_be_bytes()); // dimension length
        buf.extend_from_slice(&1i32.to_be_bytes()); // lower bound
    }

    for elem in &arr.elements {
        match value_to_binary(elem) {
            None => buf.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(bytes) => {
                let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
                buf.extend_from_slice(&len.to_be_bytes());
                buf.extend_from_slice(&bytes);
            }
        }
    }

    buf
}

/// Largest element count a binary array header may declare — 18.6's `MaxArraySize`
/// (`MaxAllocSize / sizeof(Datum)`), printed by `ArrayGetNItems`'s refusal.
const MAX_ARRAY_SIZE: i32 = 134_217_727;

/// A cursor over one binary value's bytes — the counterpart of 18.6's `StringInfo`
/// read through `pq_getmsg*`.
///
/// Each reader method answers the refusal the 18.6 reader it stands in for raises when
/// the buffer runs out: [`Self::byte`] is `pq_getmsgbyte`'s
/// `08P01 no data left in message`, and the multi-byte readers are
/// `pq_copymsgbytes`'s `08P01 insufficient data left in message`. A type's decoder
/// (its `*recv`) takes what it reads and leaves the cursor where it stopped; whether
/// anything is left over is the **caller's** question — `value_from_binary` for a
/// Bind parameter, `decode_binary_array` for an element.
#[derive(Debug)]
pub struct BinaryReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> BinaryReader<'a> {
    /// A reader positioned at the start of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    /// The number of bytes not yet read.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.cursor
    }

    /// True once every byte has been read.
    #[must_use]
    pub const fn is_exhausted(&self) -> bool {
        self.remaining() == 0
    }

    /// One byte, as `pq_getmsgbyte` reads it.
    ///
    /// # Errors
    ///
    /// [`Error::NoDataLeft`] when the buffer is exhausted.
    pub fn byte(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.cursor).ok_or(Error::NoDataLeft)?;
        self.cursor += 1;
        Ok(b)
    }

    /// The next `n` bytes, as `pq_getmsgbytes` / `pq_copymsgbytes` read them.
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than `n` bytes remain.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(Error::InsufficientDataLeft);
        }
        let out = &self.bytes[self.cursor..self.cursor + n];
        self.cursor += n;
        Ok(out)
    }

    /// A fixed-size field, read through [`Self::bytes`].
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than `N` bytes remain.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let field = self.bytes(N)?;
        <[u8; N]>::try_from(field).map_err(|_| Error::InsufficientDataLeft)
    }

    /// A big-endian `i16` (`pq_getmsgint(buf, 2)`).
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than 2 bytes remain.
    pub fn i16(&mut self) -> Result<i16> {
        self.array().map(i16::from_be_bytes)
    }

    /// A big-endian `u16` (`pq_getmsgint(buf, 2)` kept unsigned).
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than 2 bytes remain.
    pub fn u16(&mut self) -> Result<u16> {
        self.array().map(u16::from_be_bytes)
    }

    /// A big-endian `i32` (`pq_getmsgint(buf, 4)`).
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than 4 bytes remain.
    pub fn i32(&mut self) -> Result<i32> {
        self.array().map(i32::from_be_bytes)
    }

    /// A big-endian `i64` (`pq_getmsgint64`).
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when fewer than 8 bytes remain.
    pub fn i64(&mut self) -> Result<i64> {
        self.array().map(i64::from_be_bytes)
    }

    /// Everything not yet read — the `buf->len - buf->cursor` length `textrecv`,
    /// `bytearecv` and `jsonb_recv` hand their reader, which cannot run short.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = &self.bytes[self.cursor..];
        self.cursor = self.bytes.len();
        out
    }
}

/// Decode a binary-format Bind parameter into a `Value` of the given type.
///
/// The inverse of `value_to_binary` / `encode_array_binary`, read the way 18.6's
/// `exec_bind_message` reads it: the type's reader (`recv_binary`, standing in for the
/// type's `*recv`) runs over the payload, and a payload it did not consume to the last
/// byte is `22P03 incorrect binary data format in bind parameter <param_number>`.
/// `param_number` is the parameter's 1-based position in the Bind message. The server's
/// `decode_param` dispatches the binary branch here. A SQL NULL parameter is handled by
/// the caller, not here.
///
/// # Errors
///
/// Returns an error for **some** malformations of `bytes` for `oid`, and not for
/// others — this is not a gate on everything 18.6 refuses for the same payload.
/// Among the refusals: a read past the end of the payload (`08P01`, worded by the
/// reader the type's `*recv` uses), bytes left over after the type's reader
/// (`22P03`), a `jsonb` version byte that is not `1` (`XX000`), a NUL or illegal UTF-8
/// in text (`22021`), `numeric`'s sign, scale and digit refusals (`22P03`), an array
/// element length below `-1` or past the array's end (`22P03`), an array element its
/// own reader did not consume (`22P03`), an array dimension length `ArrayGetNItems`
/// refuses (`54000`), and an `ndim` outside `{0, 1}` (`0A000`). A temporal value whose
/// reader reads its full width and finds it out of range is this decoder's own
/// `22000`. A binary array header's flags word, lower bound, and an element OID other
/// than an array type pglet recognises are not checked, so a malformed header can still
/// decode to a **wrong** value (`EDGES.md` row 140).
pub fn value_from_binary(bytes: &[u8], oid: TypeOid, param_number: usize) -> Result<Value> {
    let mut buf = BinaryReader::new(bytes);
    let value = recv_binary(&mut buf, oid)?;
    if !buf.is_exhausted() {
        return Err(Error::IncorrectBinaryParamFormat {
            param: param_number,
        });
    }
    Ok(value)
}

/// Read one value of type `oid` from `buf` — the type's `*recv`, leaving the cursor
/// where that reader stops. A leftover is the caller's to refuse.
fn recv_binary(buf: &mut BinaryReader<'_>, oid: TypeOid) -> Result<Value> {
    match oid {
        // `boolrecv` reads through `pq_getmsgbyte`, whose exhausted-buffer sentence is
        // `no data left in message`; the other fixed-width readers below go through
        // `pq_getmsgint`, `pq_getmsgint64` or `pq_getmsgbytes`, which say
        // `insufficient data left in message`.
        TypeOid::Bool => Ok(Value::Bool(buf.byte()? != 0)),
        TypeOid::Int2 => Ok(Value::Int2(buf.i16()?)),
        TypeOid::Int4 => Ok(Value::Int4(buf.i32()?)),
        TypeOid::Int8 => Ok(Value::Int8(buf.i64()?)),
        TypeOid::Float4 => Ok(Value::Float4(f32::from_be_bytes(buf.array()?))),
        TypeOid::Float8 => Ok(Value::Float8(f64::from_be_bytes(buf.array()?))),
        // `textrecv` takes its bytes through `pq_getmsgtext`'s `pg_client_to_server`,
        // so a NUL or illegal UTF-8 is the server-encoding `22021`, not a malformed
        // parameter; an array's text elements reach this arm one at a time.
        TypeOid::Text | TypeOid::Varchar => Ok(Value::Text(crate::encoding::text_from_utf8(
            buf.rest().to_vec(),
        )?)),
        TypeOid::Bytea => Ok(Value::Bytea(buf.rest().to_vec())),
        TypeOid::Uuid => Ok(Value::Uuid(buf.array()?)),
        // `timestamp_recv` / `date_recv` read `i64::MIN` / `i64::MAX` and `i32::MIN` /
        // `i32::MAX` as `-infinity` / `infinity` and range-check every other count.
        // Every such count outside the domain is `22008 timestamp out of range` /
        // `date out of range` — 18.6's sentence for a count outside its own range, and
        // the domain's refusal for one inside 18.6's range but outside pglet's (a year
        // before 1 or after 9999). The count conversions are overflow-free, so an
        // extreme client-supplied value fails loud rather than panicking.
        TypeOid::Timestamp | TypeOid::Timestamptz => super::PgTimestamp::from_pg_micros(buf.i64()?)
            .ok_or(Error::TemporalOutOfRange("timestamp"))?
            .try_map(super::check_timestamp_domain)
            .map(Value::Timestamp),
        TypeOid::Date => super::PgDate::from_pg_days(buf.i32()?)
            .ok_or(Error::TemporalOutOfRange("date"))?
            .try_map(super::check_date_domain)
            .map(Value::Date),
        TypeOid::Time => {
            // Postgres `time_recv`: a bare int64 of microseconds since midnight,
            // range-checked `0 ≤ x ≤ USECS_PER_DAY` (the `24:00:00` endpoint).
            let us = buf.i64()?;
            if !(0..=86_400_000_000).contains(&us) {
                return Err(decode_err("time"));
            }
            Ok(Value::Time(us))
        }
        TypeOid::Numeric => super::PgNumeric::recv(buf).map(Value::Numeric),
        TypeOid::Interval => super::PgInterval::recv(buf).map(Value::Interval),
        TypeOid::Jsonb => {
            // `jsonb_recv` reads the first byte **unconditionally** as a version
            // (`pq_getmsgint(buf, 1)`, an unsigned byte) and only then takes the rest
            // as the JSON text (`pq_getmsgtext`). So the split runs ahead of the
            // server-encoding scan, which is 18.6's own order and is what keeps a
            // `0x00` or high leading byte a *version number* rather than a byte the
            // encoding scan gets to judge; and there is no such thing as a payload with no
            // version byte, whatever the sender meant by its first byte.
            //
            // `pq_getmsgint` reads that byte through `pq_copymsgbytes`, so an empty
            // payload is `insufficient data left in message` — the multi-byte reader's
            // sentence at a one-byte width, not `pq_getmsgbyte`'s.
            let [version] = buf.array::<1>()?;
            if version != 1 {
                return Err(Error::UnsupportedJsonbVersion { version });
            }
            let text = crate::encoding::text_from_utf8(buf.rest().to_vec())?;
            parse_jsonb(&text).map(Value::Jsonb)
        }
        _ if oid.is_array() => decode_binary_array(buf, oid),
        // Unknown scalar type: fail loud rather than silently decoding as TEXT —
        // the same "correct-or-loud, never silently-wrong" contract the
        // type-resolution paths enforce (`UnsupportedType`, SQLSTATE 42704). The
        // shape-free catalog types (`name`, `oid`, `"char"`) have no arm above and
        // land here too.
        _ => Err(Error::UnsupportedType {
            type_name: format!("{oid:?}"),
        }),
    }
}

/// A malformed-parameter error for the given type label.
fn decode_err(what: &str) -> Error {
    Error::Execution(format!("invalid binary {what} parameter"))
}

/// Decode a one-dimensional array in Postgres binary array format (the inverse
/// of `encode_array_binary`) — `array_recv` and `ReadArrayBinary`.
///
/// The dimension length is gated where `ArrayGetNItems` gates it (`54000` for a
/// negative length or one above `MAX_ARRAY_SIZE`) and then **spent**: exactly that many
/// elements are read, and whatever follows them is left for the caller's leftover
/// check. Each element's length field is checked as `ReadArrayBinary` checks it —
/// below `-1` or past the bytes left is `22P03 insufficient data left in message` —
/// and each element is read by its type's reader over its own bytes, a leftover there
/// being `22P03 improper binary format in array element <i>` (1-based, NULLs counted).
fn decode_binary_array(buf: &mut BinaryReader<'_>, oid: TypeOid) -> Result<Value> {
    let inferred_elem = oid.array_element().ok_or_else(|| decode_err("array"))?;

    let ndim = buf.i32()?;
    let _has_nulls = buf.i32()?;
    let header_elem_oid = buf.i32()?;
    // The element type the client encoded; fall back to the inferred element
    // type if we don't recognize the wire OID.
    let elem_type = TypeOid::from_oid(header_elem_oid).unwrap_or(inferred_elem);
    // A header naming an **array** type pglet recognises is refused here, before the
    // dimensions are read, as `array_recv` refuses it: every array OID pglet recognises
    // and every parameter element type it infers sit below 18.6's built-in floor, so
    // 18.6's mismatch check always fires for one. Refusing it is also what keeps this
    // decoder from recursing: an OID pglet does not recognise falls back to the scalar
    // element type, so the element type it reads with is never an array, and a payload
    // cannot nest array headers to any depth. That fallback, and a header naming another
    // *scalar* type, are still taken at face value (`EDGES.md` row 140).
    if elem_type.is_array() {
        return Err(Error::BinaryArrayElementTypeMismatch {
            found_oid: elem_type as u32,
            found: type_oid_display_name(elem_type),
            expected_oid: inferred_elem as u32,
            expected: type_oid_display_name(inferred_elem),
        });
    }

    match ndim {
        // No dimensions to read and no elements; anything after the header is the
        // caller's leftover.
        0 => Ok(Value::Array(ArrayValue::new(elem_type, Vec::new()))),
        1 => {
            let dim_len = buf.i32()?;
            let _lower_bound = buf.i32()?;
            // `ArrayGetNItems`, before any element is read.
            if !(0..=MAX_ARRAY_SIZE).contains(&dim_len) {
                return Err(Error::ProgramLimitExceeded(format!(
                    "array size exceeds the maximum allowed ({MAX_ARRAY_SIZE})"
                )));
            }
            let count = usize::try_from(dim_len).map_err(|_| decode_err("array"))?;
            // No `with_capacity(count)`: the count is the sender's claim, and a header
            // declaring `MAX_ARRAY_SIZE` over a short payload runs out of bytes first.
            let mut elements = Vec::new();
            for index in 0..count {
                let len = buf.i32()?;
                if len == -1 {
                    elements.push(Value::Null);
                    continue;
                }
                let len = usize::try_from(len)
                    .ok()
                    .filter(|&len| len <= buf.remaining())
                    .ok_or(Error::ArrayElementLengthInvalid)?;
                let mut element = BinaryReader::new(buf.bytes(len)?);
                elements.push(recv_binary(&mut element, elem_type)?);
                if !element.is_exhausted() {
                    return Err(Error::ImproperArrayElementFormat { element: index + 1 });
                }
            }
            Ok(Value::Array(ArrayValue::new(elem_type, elements)))
        }
        _ => Err(Error::NotImplemented(
            "multi-dimensional arrays are not supported".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{value_from_binary, value_sends_as, value_to_binary, value_to_binary_with_oid};
    use crate::error::Result;
    use crate::types::{ArrayValue, TypeOid, Value};

    /// A binary value decoded as Bind parameter 1.
    fn decode(bytes: &[u8], oid: TypeOid) -> Result<Value> {
        value_from_binary(bytes, oid, 1)
    }

    /// `value_sends_as` agrees with the receive side at each type that has one: a value sends
    /// as a type exactly when that type's binary reader reads the value's bytes back as
    /// the value. Over one sample of each scalar variant and each scalar type with a
    /// receive arm, so a new pairing in either codec that the relation does not carry
    /// (or one it carries that the codecs do not) reddens here.
    #[test]
    fn value_sends_as_matches_the_binary_round_trip() {
        let samples = [
            Value::Bool(true),
            Value::Int2(5),
            Value::Int4(5),
            Value::Int8(5),
            Value::Float4(1.5),
            Value::Float8(1.5),
            Value::Text("a".into()),
            Value::Bytea(b"a".to_vec()),
            Value::Timestamp(crate::types::Infinite::Finite(
                time::macros::datetime!(2000-01-01 00:00:00 UTC),
            )),
            Value::Date(crate::types::Infinite::Finite(time::macros::date!(
                2000 - 01 - 01
            ))),
            Value::Time(5),
            Value::Numeric(crate::types::PgNumeric::parse("1.5").expect("a numeric")),
            Value::Jsonb(crate::types::JsonbValue::Bool(true)),
            Value::Uuid([7; 16]),
            Value::Interval(crate::types::PgInterval {
                months: 1,
                days: 2,
                micros: 3,
            }),
        ];
        let receivable = [
            TypeOid::Bool,
            TypeOid::Int2,
            TypeOid::Int4,
            TypeOid::Int8,
            TypeOid::Float4,
            TypeOid::Float8,
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Bytea,
            TypeOid::Timestamp,
            TypeOid::Timestamptz,
            TypeOid::Date,
            TypeOid::Time,
            TypeOid::Numeric,
            TypeOid::Jsonb,
            TypeOid::Uuid,
            TypeOid::Interval,
        ];
        for v in &samples {
            let bytes = value_to_binary(v).expect("a non-NULL sample encodes");
            for oid in receivable {
                let round_trips = decode(&bytes, oid).is_ok_and(|back| back == *v);
                assert_eq!(value_sends_as(v, oid), round_trips, "{v:?} as {oid:?}");
            }
        }
    }

    /// An array's binary header names the column's element type only over elements that
    /// are sent as that type, and the value's recorded element type otherwise, so a
    /// wrong column type does not name the header over elements it does not describe. The
    /// first two are wrong column types a column-only header would have followed (a
    /// `text[]` column over `int4` elements; a `COALESCE` typed `int4[]` over `int8`
    /// elements); the last three are what the column's type is
    /// taken for (`timestamptz` over the elements `timestamp` shares, `varchar` over
    /// `text`, an all-`NULL` array recording `text` in an `int4[]` column).
    #[test]
    fn a_binary_array_header_is_the_type_its_elements_are_sent_as() {
        let header = |recorded, elements, column: u32| {
            let arr = Value::Array(ArrayValue::new(recorded, elements));
            let column = TypeOid::from_oid(column as i32).expect("an array OID");
            let bytes = value_to_binary_with_oid(&arr, column).expect("an array encodes");
            u32::from_be_bytes(bytes[8..12].try_into().expect("a header"))
        };
        let ts = Value::Timestamp(crate::types::Infinite::Infinity);
        assert_eq!(header(TypeOid::Int4, vec![Value::Int4(5)], 1009), 23);
        assert_eq!(
            header(TypeOid::Int8, vec![Value::Null, Value::Int8(5)], 1007),
            20
        );
        assert_eq!(header(TypeOid::Timestamp, vec![ts], 1185), 1184);
        assert_eq!(
            header(TypeOid::Text, vec![Value::Text("a".into())], 1015),
            1043
        );
        assert_eq!(header(TypeOid::Text, vec![Value::Null], 1007), 23);
    }

    /// A binary text value is validated in the server encoding: `textrecv` reads it
    /// through `pq_getmsgtext`'s `pg_client_to_server`, so a NUL and an illegal byte are
    /// both `22021` naming the bytes from the failure -- measured on 18.6 for a bound
    /// `text` and a bound `jsonb`. `bytea` takes any byte, and a valid multi-byte
    /// character still decodes.
    #[test]
    fn a_binary_text_value_is_validated_in_the_server_encoding() {
        for oid in [TypeOid::Text, TypeOid::Varchar] {
            let err = decode(b"x\0y", oid).unwrap_err();
            assert_eq!(
                err.to_string(),
                "invalid byte sequence for encoding \"UTF8\": 0x00",
                "{oid:?}"
            );
            let err = decode(b"x\xe9y", oid).unwrap_err();
            assert_eq!(
                err.to_string(),
                "invalid byte sequence for encoding \"UTF8\": 0xe9 0x79",
                "{oid:?}"
            );
            assert_eq!(
                decode("x\u{e9}y".as_bytes(), oid).unwrap(),
                Value::Text("x\u{e9}y".into()),
                "{oid:?}"
            );
        }
        // The jsonb payload behind its version byte.
        let err = decode(b"\x01\"x\0y\"", TypeOid::Jsonb).unwrap_err();
        assert_eq!(err.sqlstate(), "22021", "{err}");
        // A `text[]` element reaches the text arm one element at a time.
        let array = ArrayValue::new(TypeOid::Text, vec![Value::Text("x\0y".into())]);
        let bytes = value_to_binary(&Value::Array(array)).expect("an array encodes");
        let text_array = TypeOid::from_oid(1009).expect("text[] has an OID");
        let err = decode(&bytes, text_array).unwrap_err();
        assert_eq!(err.sqlstate(), "22021", "{err}");
        // The control: bytea is bytes, NUL included.
        assert_eq!(
            decode(b"x\0y", TypeOid::Bytea).unwrap(),
            Value::Bytea(b"x\0y".to_vec())
        );
    }

    /// A binary `jsonb` payload's **first byte is its version**, read unconditionally as
    /// 18.6's `jsonb_recv` reads it (`pq_getmsgint(buf, 1)`, an unsigned byte), and only
    /// `1` is supported -- anything else is `XX000 unsupported jsonb version number <n>`
    /// over that byte's unsigned decimal value. So there is no payload with "no version
    /// byte": its first byte is the version whatever the sender meant by it.
    ///
    /// Each case's SQLSTATE and message were measured on 18.6 over a raw
    /// extended-protocol client binding the same bytes. Before the split moved ahead of
    /// the server-encoding scan, what a payload earned was whatever the **unstripped**
    /// buffer earned from the encoding scan and the jsonb reader, its pseudo-version byte
    /// read as part of the JSON text: a silent accept wherever that buffer still parsed as
    /// JSON; `22021` where the scan met `0x00`, or a high byte the text behind it left
    /// illegal UTF-8; and, wherever the reader was reached, whichever of the reader's own
    /// codes that text earned -- `22P02` over a `{"a":1}` body, `22P05` and `22003` over
    /// bodies the reader parses and then refuses. That is the reader's whole verdict
    /// space, so no count of answers bounds the old arm (`EDGES.md` row 138).
    #[test]
    fn a_binary_jsonb_payloads_first_byte_is_its_version() {
        // The control: version `1`, and the remainder -- never the whole buffer -- is
        // the JSON text.
        let decoded = decode(b"\x01{\"a\":1}", TypeOid::Jsonb).unwrap();
        match decoded {
            Value::Jsonb(j) => assert_eq!(j.to_json_string(), "{\"a\": 1}"),
            other => panic!("expected a jsonb value: {other:?}"),
        }

        // (payload, the version its first byte spells).
        let cases: [(&[u8], u8); 5] = [
            (b"{\"a\":1}", 123),     // no version byte: the `{` is the version
            (b"123", 49),            // a bare number: the `1`
            (b"\x00{\"a\":1}", 0),   // the low end -- was the encoding scan's `22021`
            (b"\x02{\"a\":1}", 2),   // the next version 18.6 does not define
            (b"\xff{\"a\":1}", 255), // unsigned: `255`, never `-1`
        ];
        for (payload, version) in cases {
            let err = decode(payload, TypeOid::Jsonb).unwrap_err();
            assert_eq!(err.sqlstate(), "XX000", "{payload:?}: {err}");
            assert_eq!(
                err.to_string(),
                format!("unsupported jsonb version number {version}"),
                "{payload:?}"
            );
        }

        // An empty payload has no first byte to read. `pq_getmsgint(buf, 1)` reads it
        // through `pq_copymsgbytes`, so 18.6 answers the multi-byte reader's
        // `08P01 insufficient data left in message` before any version logic -- not
        // `pq_getmsgbyte`'s `no data left in message`, whatever the width.
        let err = decode(b"", TypeOid::Jsonb).unwrap_err();
        assert_eq!(err.sqlstate(), "08P01", "{err}");
        assert_eq!(err.to_string(), "insufficient data left in message");
    }

    /// A `jsonb[]` element reaches the same arm -- `decode_binary_array` calls
    /// `recv_binary` per element -- so the array route takes the version gate
    /// without a second implementation. Measured identically on 18.6. The header this
    /// helper writes is what makes that so: `decode_binary_array` reads the element type
    /// from the **wire array header**, not from the parameter's type, so an array headed
    /// with another scalar element OID pglet recognises decodes its elements as that type
    /// and never reaches this gate -- the header's scalar OID is not checked (`EDGES.md`
    /// row 140).
    #[test]
    fn a_binary_jsonb_array_element_carries_its_own_version_byte() {
        /// A one-element binary `jsonb[]` payload wrapping `element` verbatim.
        fn one_element_array(element: &[u8]) -> Vec<u8> {
            let mut buf = Vec::new();
            buf.extend_from_slice(&1i32.to_be_bytes()); // ndim
            buf.extend_from_slice(&0i32.to_be_bytes()); // has-nulls
            buf.extend_from_slice(&(TypeOid::Jsonb as i32).to_be_bytes());
            buf.extend_from_slice(&1i32.to_be_bytes()); // dimension length
            buf.extend_from_slice(&1i32.to_be_bytes()); // lower bound
            buf.extend_from_slice(&i32::try_from(element.len()).unwrap().to_be_bytes());
            buf.extend_from_slice(element);
            buf
        }

        // An element with no version byte: the `{` is the version, as it is for a
        // scalar param.
        let err = decode(&one_element_array(b"{\"a\":1}"), TypeOid::JsonbArray).unwrap_err();
        assert_eq!(err.sqlstate(), "XX000", "{err}");
        assert_eq!(err.to_string(), "unsupported jsonb version number 123");

        // The accept control: the same element behind its version byte decodes.
        let decoded = decode(&one_element_array(b"\x01{\"a\":1}"), TypeOid::JsonbArray).unwrap();
        match decoded {
            Value::Array(arr) => assert_eq!(arr.elements.len(), 1, "{arr:?}"),
            other => panic!("expected an array value: {other:?}"),
        }
    }

    #[test]
    fn bytea_binary_format_stays_raw() {
        // The binary path is unchanged: raw bytes, no `\x` prefix.
        assert_eq!(
            value_to_binary(&Value::Bytea(vec![0xde, 0xad, 0xbe, 0xef])),
            Some(vec![0xde, 0xad, 0xbe, 0xef])
        );
    }

    // ------------------------------------------------------------------------
    // `value_from_binary` hardening — this is the decode path for binary-format
    // Bind parameters, i.e. **bytes straight off the network from an untrusted
    // client**. The contract (like the wire codec and the SQL/JSON parsers) is
    // *correct-or-loud, never panic*: a malformed length/value/array-header must
    // return `Err`, not slice-panic, overflow, or decode silently-wrong. These
    // cases pin that — the lone pre-existing test only covered the *encoder*.
    // ------------------------------------------------------------------------

    /// A well-formed value of each fixed-width scalar round-trips through
    /// encode→decode — the positive control that makes the negative cases below
    /// meaningful (a decoder that errored on *everything* would pass them too).
    #[test]
    fn fixed_width_scalars_round_trip() {
        let cases = [
            (Value::Bool(true), TypeOid::Bool),
            (Value::Int2(-12345), TypeOid::Int2),
            (Value::Int4(1_234_567), TypeOid::Int4),
            (Value::Int8(-9_876_543_210), TypeOid::Int8),
            (Value::Float4(1.5), TypeOid::Float4),
            (Value::Float8(-2.5), TypeOid::Float8),
            (Value::Uuid([0xAB; 16]), TypeOid::Uuid),
        ];
        for (v, oid) in cases {
            let bytes = value_to_binary(&v).expect("non-null encodes");
            assert_eq!(decode(&bytes, oid).unwrap(), v, "{oid:?} must round-trip");
        }
    }

    /// A DATE round-trips through encode→decode, the 2000-01-01 epoch is exactly
    /// zero days, and a known date maps to its expected day count — pinning the
    /// Postgres binary `date` wire format (i32 days from the epoch).
    #[test]
    fn date_binary_epoch_and_round_trip() {
        use time::macros::date;
        // The epoch itself is day 0.
        assert_eq!(
            value_to_binary(&Value::date(date!(2000 - 01 - 01))),
            Some(0i32.to_be_bytes().to_vec())
        );
        // One day after the epoch is day 1; one before is -1.
        assert_eq!(
            value_to_binary(&Value::date(date!(2000 - 01 - 02))),
            Some(1i32.to_be_bytes().to_vec())
        );
        assert_eq!(
            value_to_binary(&Value::date(date!(1999 - 12 - 31))),
            Some((-1i32).to_be_bytes().to_vec())
        );
        // Round-trip a spread of dates, including pre-epoch and far-future.
        for d in [
            date!(1970 - 01 - 01),
            date!(2024 - 06 - 15),
            date!(2000 - 02 - 29),
            date!(9999 - 12 - 31),
        ] {
            let bytes = value_to_binary(&Value::date(d)).expect("date encodes");
            assert_eq!(
                decode(&bytes, TypeOid::Date).unwrap(),
                Value::date(d),
                "{d} must round-trip"
            );
        }
    }

    /// A TIME round-trips through encode→decode as a bare int64 µs, the `24:00:00`
    /// endpoint (`USECS_PER_DAY`) is accepted, and an out-of-range value fails loud —
    /// pinning the Postgres binary `time` wire format (`time_send`/`time_recv`).
    #[test]
    fn time_binary_endpoint_and_round_trip() {
        // Midnight is 0; the 24:00:00 endpoint is exactly USECS_PER_DAY.
        assert_eq!(
            value_to_binary(&Value::Time(0)),
            Some(0i64.to_be_bytes().to_vec())
        );
        assert_eq!(
            value_to_binary(&Value::Time(86_400_000_000)),
            Some(86_400_000_000i64.to_be_bytes().to_vec())
        );
        // Round-trip a spread of times, including the endpoint and sub-second µs.
        for us in [0i64, 1, 45_015_123_456, 86_399_999_999, 86_400_000_000] {
            let bytes = value_to_binary(&Value::Time(us)).expect("time encodes");
            assert_eq!(
                decode(&bytes, TypeOid::Time).unwrap(),
                Value::Time(us),
                "{us} must round-trip"
            );
        }
        // Out of range (negative or past the endpoint) fails loud.
        assert!(decode(&(-1i64).to_be_bytes(), TypeOid::Time).is_err());
        assert!(decode(&86_400_000_001i64.to_be_bytes(), TypeOid::Time).is_err());
    }

    /// A fixed-width value is read to its type's width and no further, as 18.6's
    /// `exec_bind_message` reads it: short of the width is the message reader's `08P01`
    /// (`boolrecv`'s `pq_getmsgbyte` says `no data left in message`, every other reader
    /// here `insufficient data left in message`), and past it is
    /// `22P03 incorrect binary data format in bind parameter <n>`. Each width, at `-1`,
    /// `0` and `+1`, measured on 18.6 over a raw extended-protocol client.
    #[test]
    fn a_fixed_width_value_is_read_to_its_width_and_no_further() {
        let cases = [
            (TypeOid::Bool, 1),
            (TypeOid::Int2, 2),
            (TypeOid::Int4, 4),
            (TypeOid::Int8, 8),
            (TypeOid::Float4, 4),
            (TypeOid::Float8, 8),
            (TypeOid::Uuid, 16),
            (TypeOid::Date, 4),
            (TypeOid::Time, 8),
            (TypeOid::Timestamp, 8),
            (TypeOid::Timestamptz, 8),
            (TypeOid::Interval, 16),
        ];
        for (oid, width) in cases {
            let short = if oid == TypeOid::Bool {
                "no data left in message"
            } else {
                "insufficient data left in message"
            };
            for len in [0, width - 1] {
                let err = decode(&vec![0; len], oid).unwrap_err();
                assert_eq!(err.sqlstate(), "08P01", "{oid:?} at {len}: {err}");
                assert_eq!(err.to_string(), short, "{oid:?} at {len}");
            }
            // The exact width decodes -- zero is in range for every type here.
            assert!(decode(&vec![0; width], oid).is_ok(), "{oid:?} at {width}");
            let err = decode(&vec![0; width + 1], oid).unwrap_err();
            assert_eq!(err.sqlstate(), "22P03", "{oid:?} over-long: {err}");
            assert_eq!(
                err.to_string(),
                "incorrect binary data format in bind parameter 1",
                "{oid:?} over-long"
            );
        }
        // The number is the caller's, never a constant.
        let err = value_from_binary(&[0; 5], TypeOid::Int4, 7).unwrap_err();
        assert_eq!(
            err.to_string(),
            "incorrect binary data format in bind parameter 7"
        );
    }

    /// A range verdict is the type's reader's own, so it comes **before** the leftover
    /// check: an out-of-range `time` with a trailing byte answers whatever the same
    /// value answers with no tail, and not `22P03`. 18.6 answers
    /// `22008 time out of range` at both payloads; which code pglet's verdict carries is
    /// not this test's to pin, only that the tail does not change it.
    #[test]
    fn a_type_readers_own_verdict_precedes_the_leftover_check() {
        let exact = 86_400_000_001i64.to_be_bytes().to_vec();
        let mut tailed = exact.clone();
        tailed.push(0);
        let verdict = decode(&exact, TypeOid::Time).unwrap_err();
        let err = decode(&tailed, TypeOid::Time).unwrap_err();
        assert_ne!(err.sqlstate(), "22P03", "{err}");
        assert_eq!(
            (err.sqlstate(), err.to_string()),
            (verdict.sqlstate(), verdict.to_string())
        );
    }

    /// The text-shaped readers take everything that is left, so no payload is too long
    /// or too short for them -- an empty `text`, `varchar` and `bytea` all decode, on
    /// both engines.
    #[test]
    fn a_rest_of_buffer_reader_consumes_any_length() {
        assert_eq!(
            decode(b"", TypeOid::Text).unwrap(),
            Value::Text(String::new())
        );
        assert_eq!(
            decode(b"", TypeOid::Varchar).unwrap(),
            Value::Text(String::new())
        );
        assert_eq!(
            decode(b"", TypeOid::Bytea).unwrap(),
            Value::Bytea(Vec::new())
        );
    }

    /// An extreme binary timestamp value must fail loud, not overflow. The shift to
    /// the Postgres epoch used to be `micros + OFFSET` in `i64`, which panics under
    /// debug overflow-checks (and silently wraps in release) for `i64::MAX` — a
    /// malformed param crashing the decoder. Regression guard for the i128 shift. The
    /// extremes themselves are the infinity sentinels
    /// (`binary_temporal_sentinels_are_the_infinities`), so the neighbours are the probe.
    #[test]
    fn extreme_timestamp_values_fail_loud_not_overflow() {
        for &micros in &[i64::MAX - 1, i64::MIN + 1, i64::MAX / 2, i64::MIN / 2] {
            for oid in [TypeOid::Timestamp, TypeOid::Timestamptz] {
                let r = decode(&micros.to_be_bytes(), oid);
                assert!(
                    r.is_err(),
                    "{oid:?} from {micros} must be a loud error (out of range), got {r:?}"
                );
            }
        }
    }

    /// The binary `date` / `timestamp` / `timestamptz` readers hold the domain: each
    /// edge accepted, one day or one microsecond past each refused, and every count
    /// outside it — 18.6's range ends included, its `infinity` sentinels not — the
    /// reader's `22008` sentence. Measured on 18.6 over a raw extended-protocol client:
    /// the counts below 4714-11-24 BC, past 5874897-12-31 (a date) and past 294276 (a
    /// timestamp) answer these sentences there too, and the counts between accept.
    #[test]
    fn binary_temporal_readers_hold_the_domain() {
        use time::macros::{date, datetime};
        const DAY: i64 = 86_400_000_000;
        let refused = |bytes: &[u8], oid: TypeOid, sentence: &str| {
            let e = decode(bytes, oid).expect_err("out of domain");
            assert_eq!(
                (e.sqlstate(), e.to_string().as_str()),
                ("22008", sentence),
                "{oid:?}"
            );
        };
        // 0001-01-01 is day -730119 from the 2000-01-01 epoch; 10000-01-01 is 2921940.
        assert_eq!(
            decode(&(-730_119i32).to_be_bytes(), TypeOid::Date).unwrap(),
            Value::date(date!(0001 - 01 - 01))
        );
        assert_eq!(
            decode(&2_921_939i32.to_be_bytes(), TypeOid::Date).unwrap(),
            Value::date(date!(9999 - 12 - 31))
        );
        for days in [
            -730_120,
            2_921_940,
            -2_451_546,
            2_145_031_949,
            i32::MAX - 1,
            i32::MIN + 1,
        ] {
            refused(&days.to_be_bytes(), TypeOid::Date, "date out of range");
        }
        for oid in [TypeOid::Timestamp, TypeOid::Timestamptz] {
            assert_eq!(
                decode(&(-730_119 * DAY).to_be_bytes(), oid).unwrap(),
                Value::timestamp(datetime!(0001-01-01 00:00:00 UTC))
            );
            assert_eq!(
                decode(&(2_921_940 * DAY - 1).to_be_bytes(), oid).unwrap(),
                Value::timestamp(datetime!(9999-12-31 23:59:59.999999 UTC))
            );
            for micros in [
                -730_119 * DAY - 1,
                2_921_940 * DAY,
                i64::MAX - 1,
                i64::MIN + 1,
            ] {
                refused(&micros.to_be_bytes(), oid, "timestamp out of range");
            }
        }
    }

    /// `DATE_NOBEGIN` / `DATE_NOEND` (`i32::MIN` / `i32::MAX`) and `TIMESTAMP_NOBEGIN` /
    /// `TIMESTAMP_NOEND` (`i64::MIN` / `i64::MAX`) are `-infinity` / `infinity` on
    /// receive and on send, as `date_recv` / `timestamp_recv` and their `_send`s read
    /// and write them — measured on 18.6, whose binary `'infinity'::date` is
    /// `7f ff ff ff`.
    #[test]
    fn binary_temporal_sentinels_are_the_infinities() {
        use super::super::{ArrayValue, Infinite};
        for (days, d) in [
            (i32::MIN, Infinite::NegInfinity),
            (i32::MAX, Infinite::Infinity),
        ] {
            let bytes = days.to_be_bytes();
            assert_eq!(decode(&bytes, TypeOid::Date).unwrap(), Value::Date(d));
            assert_eq!(value_to_binary(&Value::Date(d)), Some(bytes.to_vec()));
        }
        for (micros, ts) in [
            (i64::MIN, Infinite::NegInfinity),
            (i64::MAX, Infinite::Infinity),
        ] {
            let bytes = micros.to_be_bytes();
            for oid in [TypeOid::Timestamp, TypeOid::Timestamptz] {
                assert_eq!(decode(&bytes, oid).unwrap(), Value::Timestamp(ts));
            }
            assert_eq!(value_to_binary(&Value::Timestamp(ts)), Some(bytes.to_vec()));
        }
        // An array element takes the same reader and the same encoder.
        let arr = Value::Array(ArrayValue::new(
            TypeOid::Date,
            vec![
                Value::Date(Infinite::Infinity),
                Value::Null,
                Value::Date(Infinite::NegInfinity),
            ],
        ));
        let bytes = value_to_binary(&arr).expect("array encodes");
        assert_eq!(decode(&bytes, TypeOid::DateArray).unwrap(), arr);
    }

    /// Invalid UTF-8 in a text/varchar/jsonb param fails loud, never producing a
    /// lossy or panicking decode.
    #[test]
    fn invalid_utf8_text_and_jsonb_error() {
        let bad_utf8 = [0xFF, 0xFE, 0x80];
        for oid in [TypeOid::Text, TypeOid::Varchar] {
            assert!(decode(&bad_utf8, oid).is_err(), "{oid:?} utf8");
        }
        // JSONB binary is `[1]`-prefixed; bad UTF-8 after the version byte errors.
        assert!(decode(&[1, 0xFF, 0xFE], TypeOid::Jsonb).is_err());
    }

    /// A version-prefixed but syntactically-broken JSONB payload fails the parse
    /// (delegates to the JSON parser's loud failure), not a partial/empty decode.
    #[test]
    fn malformed_jsonb_payload_errors() {
        assert!(decode(b"\x01{not json", TypeOid::Jsonb).is_err());
    }

    /// Bool is exactly one byte (zero = false, nonzero = true), matching Postgres
    /// `boolrecv`; an empty or over-length buffer is malformed and fails loud.
    #[test]
    fn bool_decode_requires_exactly_one_byte() {
        assert_eq!(decode(&[0], TypeOid::Bool).unwrap(), Value::Bool(false));
        assert_eq!(decode(&[1], TypeOid::Bool).unwrap(), Value::Bool(true));
        assert_eq!(decode(&[42], TypeOid::Bool).unwrap(), Value::Bool(true));
        assert!(decode(&[], TypeOid::Bool).is_err(), "empty bool errors");
        assert!(
            decode(&[1, 2], TypeOid::Bool).is_err(),
            "over-length bool errors"
        );
    }

    // --- binary array header fuzzing (decode_binary_array) ------------------

    /// A truncated array header (fewer than the 5 leading i32 fields) is the message
    /// reader's `08P01`.
    #[test]
    fn truncated_array_header_errors() {
        // ndim=1 declared, but the stream ends mid-header.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1i32.to_be_bytes()); // ndim
        bytes.extend_from_slice(&0i32.to_be_bytes()); // has_nulls
        // missing element OID + dim length + lower bound
        assert_eq!(
            decode(&bytes, TypeOid::Int4Array).unwrap_err().sqlstate(),
            "08P01"
        );
    }

    /// An element whose declared length runs past the buffer end fails loud, never
    /// slicing out of bounds (`a_binary_array_element_is_read_to_its_own_length` pins
    /// the code).
    #[test]
    fn array_element_length_past_end_errors() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1i32.to_be_bytes()); // ndim = 1
        bytes.extend_from_slice(&0i32.to_be_bytes()); // has_nulls
        bytes.extend_from_slice(&(TypeOid::Int4 as i32).to_be_bytes()); // elem OID
        bytes.extend_from_slice(&1i32.to_be_bytes()); // dim length
        bytes.extend_from_slice(&1i32.to_be_bytes()); // lower bound
        bytes.extend_from_slice(&100i32.to_be_bytes()); // element len = 100…
        bytes.extend_from_slice(&[0u8; 4]); // …but only 4 bytes follow
        assert!(decode(&bytes, TypeOid::Int4Array).is_err());
    }

    /// A multi-dimensional array (ndim ≥ 2) is rejected loud (unsupported), not
    /// mis-decoded.
    #[test]
    fn multi_dim_array_rejected() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i32.to_be_bytes()); // ndim = 2
        bytes.extend_from_slice(&0i32.to_be_bytes()); // has_nulls
        bytes.extend_from_slice(&(TypeOid::Int4 as i32).to_be_bytes());
        assert!(decode(&bytes, TypeOid::Int4Array).is_err());
    }

    /// An empty array (ndim 0) decodes to an empty `Value::Array`, and a NULL
    /// element (len -1) decodes to `Value::Null` — the two non-error edge shapes.
    #[test]
    fn empty_and_null_element_arrays_decode() {
        // ndim 0 → empty array.
        let mut empty = Vec::new();
        empty.extend_from_slice(&0i32.to_be_bytes()); // ndim = 0
        empty.extend_from_slice(&0i32.to_be_bytes()); // has_nulls
        empty.extend_from_slice(&(TypeOid::Int4 as i32).to_be_bytes());
        match decode(&empty, TypeOid::Int4Array).unwrap() {
            Value::Array(a) => assert!(a.elements.is_empty()),
            other => panic!("expected empty array, got {other:?}"),
        }

        // One NULL element (declared length -1).
        let mut with_null = Vec::new();
        with_null.extend_from_slice(&1i32.to_be_bytes()); // ndim = 1
        with_null.extend_from_slice(&1i32.to_be_bytes()); // has_nulls
        with_null.extend_from_slice(&(TypeOid::Int4 as i32).to_be_bytes());
        with_null.extend_from_slice(&1i32.to_be_bytes()); // dim length
        with_null.extend_from_slice(&1i32.to_be_bytes()); // lower bound
        with_null.extend_from_slice(&(-1i32).to_be_bytes()); // NULL element
        match decode(&with_null, TypeOid::Int4Array).unwrap() {
            Value::Array(a) => assert_eq!(a.elements, vec![Value::Null]),
            other => panic!("expected [NULL], got {other:?}"),
        }
    }

    /// A binary `numeric` body: `ndigits`, weight, sign and scale as written (each
    /// 16 bits), then each digit.
    fn numeric_body(ndigits: u16, weight: i16, sign: u16, dscale: u16, digits: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&ndigits.to_be_bytes());
        out.extend_from_slice(&weight.to_be_bytes());
        out.extend_from_slice(&sign.to_be_bytes());
        out.extend_from_slice(&dscale.to_be_bytes());
        for digit in digits {
            out.extend_from_slice(&digit.to_be_bytes());
        }
        out
    }

    /// Assert `payload` decoded as `oid` is refused with exactly `code` and `message`.
    fn assert_refused(payload: &[u8], oid: TypeOid, code: &str, message: &str) {
        let err = decode(payload, oid).unwrap_err();
        assert_eq!(
            (err.sqlstate(), err.to_string().as_str()),
            (code, message),
            "{oid:?} {payload:02x?}"
        );
    }

    const PARAM_LEFTOVER: &str = "incorrect binary data format in bind parameter 1";
    const INSUFFICIENT: &str = "insufficient data left in message";

    /// A binary `numeric` is read in `numeric_recv`'s order -- digit count (unsigned),
    /// weight, sign, scale, digits -- and each refusal is that reader's: a field past
    /// the end is `08P01`, the sign, scale and digit checks are `22P03`, and bytes after
    /// the last digit are the bind path's `22P03`. Every payload here was measured on
    /// 18.6 over a raw extended-protocol client.
    #[test]
    fn a_binary_numeric_is_read_in_numeric_recvs_order() {
        let n = TypeOid::Numeric;
        // Controls: `1`, `9999`, a `NaN` whose header declares a digit (18.6 reads and
        // ignores it), and the largest scale the mask admits.
        assert_eq!(
            decode(&numeric_body(1, 0, 0, 0, &[1]), n).unwrap(),
            Value::Numeric(crate::types::PgNumeric::parse("1").unwrap())
        );
        assert_eq!(
            decode(&numeric_body(1, 0, 0, 0, &[9999]), n).unwrap(),
            Value::Numeric(crate::types::PgNumeric::parse("9999").unwrap())
        );
        assert_eq!(
            decode(&numeric_body(1, 0, 0xC000, 0, &[1]), n).unwrap(),
            Value::Numeric(crate::types::PgNumeric::NaN)
        );
        assert!(decode(&numeric_body(0, 0, 0, 0x3FFF, &[]), n).is_ok());

        // Trailing bytes after a complete body, at a number and at `NaN`.
        for tail in [1usize, 2, 4] {
            let mut body = numeric_body(1, 0, 0, 0, &[1]);
            body.extend(std::iter::repeat_n(0, tail));
            assert_refused(&body, n, "22P03", PARAM_LEFTOVER);
        }
        let mut nan = numeric_body(0, 0, 0xC000, 0, &[]);
        nan.extend_from_slice(&[0, 1]);
        assert_refused(&nan, n, "22P03", PARAM_LEFTOVER);

        // Short: empty, one byte, a six- and seven-byte header, a digit short, and a
        // digit count of `0xFFFF` read as 65535 -- never as `-1`.
        let one = numeric_body(1, 0, 0, 0, &[1]);
        let header = numeric_body(0, 0, 0, 0, &[]);
        for short in [&[][..], &[0][..], &one[..6], &header[..7]] {
            assert_refused(short, n, "08P01", INSUFFICIENT);
        }
        assert_refused(&numeric_body(2, 0, 0, 0, &[1]), n, "08P01", INSUFFICIENT);
        assert_refused(
            &numeric_body(2, 0, 0xC000, 0, &[1]),
            n,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(
            &numeric_body(0xFFFF, 0, 0, 0, &[]),
            n,
            "08P01",
            INSUFFICIENT,
        );

        // The sign is checked before the scale and before any digit is read: it beats a
        // 65535-digit shortfall and a trailing tail.
        let bad_sign = "invalid sign in external \"numeric\" value";
        assert_refused(&numeric_body(1, 0, 0x1234, 0, &[1]), n, "22P03", bad_sign);
        assert_refused(
            &numeric_body(0xFFFF, 0, 0x1234, 0, &[]),
            n,
            "22P03",
            bad_sign,
        );
        let mut tailed = numeric_body(1, 0, 0x1234, 0, &[1]);
        tailed.extend_from_slice(&[0, 0]);
        assert_refused(&tailed, n, "22P03", bad_sign);
        assert_refused(
            &numeric_body(1, 0, 0x1234, 0x4000, &[1]),
            n,
            "22P03",
            bad_sign,
        );

        // The scale: any bit outside `0x3FFF`, ahead of a digit shortfall and at `NaN`.
        let bad_scale = "invalid scale in external \"numeric\" value";
        assert_refused(&numeric_body(0, 0, 0, 0x4000, &[]), n, "22P03", bad_scale);
        assert_refused(&numeric_body(1, 0, 0, 0xFFFF, &[1]), n, "22P03", bad_scale);
        assert_refused(&numeric_body(1, 0, 0, 0x4000, &[]), n, "22P03", bad_scale);
        assert_refused(
            &numeric_body(0, 0, 0xC000, 0x4000, &[]),
            n,
            "22P03",
            bad_scale,
        );

        // A digit outside `0..=9999`, read signed, at a number and at `NaN`, and ahead
        // of a later digit's shortfall.
        let bad_digit = "invalid digit in external \"numeric\" value";
        assert_refused(&numeric_body(1, 0, 0, 0, &[10000]), n, "22P03", bad_digit);
        assert_refused(&numeric_body(1, 0, 0, 0, &[0xFFFF]), n, "22P03", bad_digit);
        assert_refused(
            &numeric_body(1, 0, 0xC000, 0, &[10000]),
            n,
            "22P03",
            bad_digit,
        );
        assert_refused(&numeric_body(2, 0, 0, 0, &[10000]), n, "22P03", bad_digit);
    }

    /// `Infinity` / `-Infinity` pass 18.6's sign check and are accepted there;
    /// `PgNumeric` has no infinity, so a complete one is refused here with the reader's
    /// `22P02`. The refusal sits after the fields 18.6 reads, so a malformed infinity
    /// body earns 18.6's own answer first.
    #[test]
    fn a_binary_numeric_infinity_is_refused_after_its_fields_are_read() {
        let n = TypeOid::Numeric;
        let unrepresentable = "invalid input syntax for type numeric: \"<binary>\"";
        for sign in [0xD000, 0xF000] {
            assert_refused(
                &numeric_body(0, 0, sign, 0, &[]),
                n,
                "22P02",
                unrepresentable,
            );
            assert_refused(&numeric_body(1, 0, sign, 0, &[]), n, "08P01", INSUFFICIENT);
            assert_refused(
                &numeric_body(0, 0, sign, 0x4000, &[]),
                n,
                "22P03",
                "invalid scale in external \"numeric\" value",
            );
        }
    }

    /// A one-dimensional binary array header over `elements` (each already
    /// length-prefixed), declaring `dim_len` elements of type `elem_oid`.
    fn array_body(elem_oid: TypeOid, dim_len: i32, elements: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for field in [1, 0, elem_oid as i32, dim_len, 1] {
            out.extend_from_slice(&i32::to_be_bytes(field));
        }
        for element in elements {
            out.extend_from_slice(element);
        }
        out
    }

    /// One element's length prefix and bytes.
    fn element(bytes: &[u8]) -> Vec<u8> {
        let mut out = i32::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    const NULL_ELEMENT: [u8; 4] = (-1i32).to_be_bytes();

    /// A binary array's dimension length is **spent**: exactly that many elements are
    /// read, and what follows them is the bind path's leftover -- never another
    /// element. Short of the count is the message reader's `08P01`. Every payload here
    /// was measured on 18.6 over a raw extended-protocol client; before this, pglet
    /// decoded the excess as further elements and the shortfall as a short array.
    #[test]
    fn a_binary_arrays_dimension_length_is_spent() {
        let text = TypeOid::TextArray;
        let a = element(b"a");
        let texts = |v: &[&str]| {
            Value::Array(ArrayValue::new(
                TypeOid::Text,
                v.iter().map(|s| Value::Text((*s).into())).collect(),
            ))
        };
        // Controls.
        assert_eq!(
            decode(&array_body(TypeOid::Text, 1, &[&a]), text).unwrap(),
            texts(&["a"])
        );
        assert_eq!(
            decode(&array_body(TypeOid::Text, 0, &[]), text).unwrap(),
            texts(&[])
        );
        let mut ndim0 = Vec::new();
        for field in [0, 0, TypeOid::Text as i32] {
            ndim0.extend_from_slice(&i32::to_be_bytes(field));
        }
        assert_eq!(decode(&ndim0, text).unwrap(), texts(&[]));
        let nulls = array_body(TypeOid::Int4, 2, &[&NULL_ELEMENT, &NULL_ELEMENT]);
        assert_eq!(
            decode(&nulls, TypeOid::Int4Array).unwrap(),
            Value::Array(ArrayValue::new(
                TypeOid::Int4,
                vec![Value::Null, Value::Null]
            ))
        );

        // A tail after the declared elements: a zero length, a NULL, a whole element.
        for tail in [
            &0i32.to_be_bytes()[..],
            &NULL_ELEMENT[..],
            &element(b"b")[..],
        ] {
            assert_refused(
                &array_body(TypeOid::Text, 1, &[&a, tail]),
                text,
                "22P03",
                PARAM_LEFTOVER,
            );
        }
        assert_refused(
            &array_body(
                TypeOid::Int4,
                1,
                &[&element(&5i32.to_be_bytes()), &element(&6i32.to_be_bytes())],
            ),
            TypeOid::Int4Array,
            "22P03",
            PARAM_LEFTOVER,
        );
        let mut nulls_tailed = nulls;
        nulls_tailed.push(0);
        assert_refused(&nulls_tailed, TypeOid::Int4Array, "22P03", PARAM_LEFTOVER);
        // A dimension length of `0` reads no element, so one present is a leftover.
        assert_refused(
            &array_body(TypeOid::Text, 0, &[&a]),
            text,
            "22P03",
            PARAM_LEFTOVER,
        );
        // `ndim = 0` reads no dimension either.
        let mut ndim0_tailed = ndim0.clone();
        ndim0_tailed.extend_from_slice(&0i32.to_be_bytes());
        assert_refused(&ndim0_tailed, text, "22P03", PARAM_LEFTOVER);

        // Short of the declared count, and a header or length field cut short.
        assert_refused(
            &array_body(TypeOid::Text, 2, &[&a]),
            text,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(
            &array_body(TypeOid::Text, 2, &[&a, &[0, 0]]),
            text,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(
            &array_body(TypeOid::Text, 1, &[])[..19],
            text,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(&ndim0[..8], text, "08P01", INSUFFICIENT);
        assert_refused(&[], text, "08P01", INSUFFICIENT);
    }

    /// `ArrayGetNItems` gates the dimension length before any element is read: a
    /// negative length, or one above `134217727`, is `54000` whatever the payload
    /// carries; `134217727` itself passes the gate and the spend then runs short.
    #[test]
    fn a_binary_arrays_dimension_length_is_gated_before_it_is_spent() {
        let too_large = "array size exceeds the maximum allowed (134217727)";
        let int4 = element(&5i32.to_be_bytes());
        for dim_len in [-1, -2, i32::MIN, 134_217_728, i32::MAX] {
            for body in [&[][..], &[&int4[..]][..]] {
                assert_refused(
                    &array_body(TypeOid::Int4, dim_len, body),
                    TypeOid::Int4Array,
                    "54000",
                    too_large,
                );
            }
        }
        assert_refused(
            &array_body(TypeOid::Int4, 134_217_727, &[]),
            TypeOid::Int4Array,
            "08P01",
            INSUFFICIENT,
        );
        // The gate runs after the lower bound is read, so a header cut off inside the
        // lower bound is the reader's `08P01` whatever the length -- measured on 18.6
        // for `-1` with no lower bound and `134217728` with two bytes of one.
        let header = array_body(TypeOid::Int4, -1, &[]);
        assert_refused(&header[..16], TypeOid::Int4Array, "08P01", INSUFFICIENT);
        let header = array_body(TypeOid::Int4, 134_217_728, &[]);
        assert_refused(&header[..18], TypeOid::Int4Array, "08P01", INSUFFICIENT);
    }

    /// Each element is read by `ReadArrayBinary`'s rules: a length below `-1` or past
    /// the bytes left is `22P03 insufficient data left in message` -- the message
    /// reader's words under a different code -- and the element's own reader runs over
    /// exactly its bytes, a leftover there being
    /// `22P03 improper binary format in array element <i>`, counted from `1` with NULL
    /// elements counted. A read past an element's own end is the reader's `08P01`.
    /// Every payload here was measured on 18.6 over a raw extended-protocol client.
    #[test]
    fn a_binary_array_element_is_read_to_its_own_length() {
        let int4 = TypeOid::Int4Array;
        let one = element(&1i32.to_be_bytes());

        // The length field: below `-1`, and past what is left.
        let array_insufficient =
            |payload: &[u8], oid| assert_refused(payload, oid, "22P03", INSUFFICIENT);
        array_insufficient(
            &array_body(TypeOid::Int4, 1, &[&(-2i32).to_be_bytes()]),
            int4,
        );
        array_insufficient(
            &array_body(TypeOid::Int4, 2, &[&one, &(-2i32).to_be_bytes()]),
            int4,
        );
        array_insufficient(
            &array_body(TypeOid::Int4, 1, &[&100i32.to_be_bytes(), &[0; 4]]),
            int4,
        );
        array_insufficient(
            &array_body(
                TypeOid::Text,
                2,
                &[&element(b"a"), &9i32.to_be_bytes(), b"b"],
            ),
            TypeOid::TextArray,
        );

        // An element its reader does not consume, numbered from 1 with NULLs counted.
        let five = element(&[0; 5]);
        let improper = |i: usize| format!("improper binary format in array element {i}");
        assert_refused(
            &array_body(TypeOid::Int4, 1, &[&element(&[0, 0, 0, 5, 0])]),
            int4,
            "22P03",
            &improper(1),
        );
        assert_refused(
            &array_body(TypeOid::Int4, 2, &[&one, &five]),
            int4,
            "22P03",
            &improper(2),
        );
        assert_refused(
            &array_body(TypeOid::Int4, 2, &[&NULL_ELEMENT, &five]),
            int4,
            "22P03",
            &improper(2),
        );
        assert_refused(
            &array_body(TypeOid::Bool, 1, &[&element(&[0, 0])]),
            TypeOid::BoolArray,
            "22P03",
            &improper(1),
        );
        let mut numeric = numeric_body(1, 0, 0, 0, &[1]);
        numeric.extend_from_slice(&[0, 7]);
        assert_refused(
            &array_body(TypeOid::Numeric, 1, &[&element(&numeric)]),
            TypeOid::NumericArray,
            "22P03",
            &improper(1),
        );

        // An element's reader running past the element's end, with that reader's
        // sentence, and the element reader's own refusals passing through.
        assert_refused(
            &array_body(TypeOid::Int4, 1, &[&element(&[0, 0, 5])]),
            int4,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(
            &array_body(TypeOid::Bool, 2, &[&NULL_ELEMENT, &element(&[])]),
            TypeOid::BoolArray,
            "08P01",
            "no data left in message",
        );
        assert_refused(
            &array_body(TypeOid::Jsonb, 2, &[&element(b"\x011"), &element(&[])]),
            TypeOid::JsonbArray,
            "08P01",
            INSUFFICIENT,
        );
        assert_refused(
            &array_body(
                TypeOid::Numeric,
                1,
                &[&element(&numeric_body(1, 0, 0x1234, 0, &[1]))],
            ),
            TypeOid::NumericArray,
            "22P03",
            "invalid sign in external \"numeric\" value",
        );

        // The leftover check runs as each element is read, not after the loop: a leftover
        // in element 2 is refused before element 3's shortfall is reached, and element
        // 2's shortfall before element 3's leftover. Both measured on 18.6.
        assert_refused(
            &array_body(TypeOid::Int4, 3, &[&NULL_ELEMENT, &five, &element(&[0; 3])]),
            int4,
            "22P03",
            &improper(2),
        );
        assert_refused(
            &array_body(TypeOid::Int4, 3, &[&NULL_ELEMENT, &element(&[0; 3]), &five]),
            int4,
            "08P01",
            INSUFFICIENT,
        );

        // Controls: a zero-length `bytea` and a `NaN`-with-digit element decode.
        assert!(
            decode(
                &array_body(TypeOid::Bytea, 1, &[&element(&[])]),
                TypeOid::ByteaArray
            )
            .is_ok()
        );
        assert!(
            decode(
                &array_body(
                    TypeOid::Numeric,
                    1,
                    &[&element(&numeric_body(1, 0, 0xC000, 0, &[1]))]
                ),
                TypeOid::NumericArray
            )
            .is_ok()
        );
    }

    /// A binary array header whose element OID names an **array** type pglet recognises
    /// is refused with
    /// `array_recv`'s `42804`, naming both types as `format_type` does, before the
    /// dimensions are read -- so at `ndim = 0`, and on a header that ends after the OID,
    /// too. Each message was measured on 18.6. Before this, the header's array OID was
    /// decoded as the element type and the decoder recursed into the element as another
    /// array, so a payload nesting headers deeply enough overflowed the server's stack.
    #[test]
    fn a_binary_array_header_naming_an_array_element_type_is_refused() {
        let mismatch = |found: u32, found_name: &str, expected: u32, expected_name: &str| {
            format!(
                "binary data has array element type {found} ({found_name}) instead of expected {expected} ({expected_name})"
            )
        };
        let x = element(b"x");
        let inner = array_body(TypeOid::Text, 1, &[&x]);
        assert_refused(
            &array_body(TypeOid::TextArray, 1, &[&element(&inner)]),
            TypeOid::TextArray,
            "42804",
            &mismatch(1009, "text[]", 25, "text"),
        );
        // Not only a header naming the parameter's own array type.
        assert_refused(
            &array_body(TypeOid::TextArray, 1, &[&element(&5i32.to_be_bytes())]),
            TypeOid::Int4Array,
            "42804",
            &mismatch(1009, "text[]", 23, "integer"),
        );
        assert_refused(
            &array_body(
                TypeOid::TimestamptzArray,
                1,
                &[&element(&0i64.to_be_bytes())],
            ),
            TypeOid::TimestamptzArray,
            "42804",
            &mismatch(
                1185,
                "timestamp with time zone[]",
                1184,
                "timestamp with time zone",
            ),
        );
        // Ahead of the dimensions: at `ndim = 0`, and over a header that stops after the
        // element OID.
        let mut header = Vec::new();
        for field in [0, 0, TypeOid::TextArray as i32] {
            header.extend_from_slice(&i32::to_be_bytes(field));
        }
        assert_refused(
            &header,
            TypeOid::TextArray,
            "42804",
            &mismatch(1009, "text[]", 25, "text"),
        );
        header[..4].copy_from_slice(&1i32.to_be_bytes());
        assert_refused(
            &header,
            TypeOid::TextArray,
            "42804",
            &mismatch(1009, "text[]", 25, "text"),
        );

        // Nesting to a depth that recursed past the stack: refused at the outermost
        // header. Built inside-out by length so the payload is linear in the depth.
        let depth = 5000;
        let mut lengths = vec![inner.len()];
        for _ in 0..depth {
            lengths.push(lengths[lengths.len() - 1] + 24);
        }
        let mut nested = Vec::with_capacity(lengths[depth]);
        for level in (0..depth).rev() {
            for field in [1, 0, TypeOid::TextArray as i32, 1, 1] {
                nested.extend_from_slice(&i32::to_be_bytes(field));
            }
            nested.extend_from_slice(&i32::try_from(lengths[level]).unwrap().to_be_bytes());
        }
        nested.extend_from_slice(&inner);
        assert_eq!(nested.len(), lengths[depth]);
        assert_refused(
            &nested,
            TypeOid::TextArray,
            "42804",
            &mismatch(1009, "text[]", 25, "text"),
        );
    }

    /// 18.6 reads the all-maximum and all-minimum `interval` field triples as `infinity`
    /// and `-infinity`; `PgInterval` has neither, so each is refused with the `22007` the
    /// text path gives the word `infinity` / `-infinity`, rather than stored as the finite
    /// interval its three fields would spell, on the scalar route and as an array element.
    /// Each triple the test lists as finite -- a sentinel with one field set to `0`, and
    /// `(0, 0, MAX)` -- is an ordinary finite interval on both engines, measured.
    #[test]
    fn a_binary_interval_infinity_is_refused_as_the_text_path_refuses_the_word() {
        let interval = |micros: i64, days: i32, months: i32| {
            let mut out = micros.to_be_bytes().to_vec();
            out.extend_from_slice(&days.to_be_bytes());
            out.extend_from_slice(&months.to_be_bytes());
            out
        };
        let plus = interval(i64::MAX, i32::MAX, i32::MAX);
        let minus = interval(i64::MIN, i32::MIN, i32::MIN);
        let syntax = |s: &str| format!("invalid input syntax for type interval: \"{s}\"");
        assert_refused(&plus, TypeOid::Interval, "22007", &syntax("infinity"));
        assert_refused(&minus, TypeOid::Interval, "22007", &syntax("-infinity"));
        assert_refused(
            &array_body(TypeOid::Interval, 1, &[&element(&plus)]),
            TypeOid::IntervalArray,
            "22007",
            &syntax("infinity"),
        );
        // Each sentinel's neighbours with one field moved off it, every field taking its
        // turn -- so a check reading fewer than all three fields refuses one of them.
        for finite in [
            interval(0, i32::MAX, i32::MAX),
            interval(i64::MAX, 0, i32::MAX),
            interval(i64::MAX, i32::MAX, 0),
            interval(0, i32::MIN, i32::MIN),
            interval(i64::MIN, 0, i32::MIN),
            interval(i64::MIN, i32::MIN, 0),
            interval(0, 0, i32::MAX),
        ] {
            assert!(decode(&finite, TypeOid::Interval).is_ok(), "{finite:02x?}");
        }
        // The text path answers the word the same way; the triple spelled as numbers is a
        // finite interval there, which this test does not assert.
        let text = crate::types::PgInterval::parse("infinity").unwrap_err();
        assert_eq!(
            (text.sqlstate(), text.to_string()),
            ("22007", syntax("infinity"))
        );
    }

    /// `numeric_recv` truncates the digits a display scale hides (`trunc_var`) toward
    /// zero, and a value that truncates to zero is a positive zero. Measured on 18.6:
    /// `1.5678` sent with scale 1 compares equal to `1.5`; `-0.5` sent with scale 0 is
    /// `0` and equal to `0`.
    #[test]
    fn a_binary_numeric_is_truncated_to_its_display_scale() {
        let n = TypeOid::Numeric;
        let numeric = |text: &str| Value::Numeric(crate::types::PgNumeric::parse(text).unwrap());
        let decoded = decode(&numeric_body(2, 0, 0, 1, &[1, 5678]), n).unwrap();
        assert_eq!(decoded, numeric("1.5"));
        let Value::Numeric(ref value) = decoded else {
            unreachable!()
        };
        assert_eq!(value.to_display_string(), "1.5");
        let negative = decode(&numeric_body(1, -1, 0x4000, 0, &[5000]), n).unwrap();
        let Value::Numeric(ref value) = negative else {
            unreachable!()
        };
        assert_eq!(value.to_display_string(), "0");
        assert!(value.is_zero());
        let small = decode(&numeric_body(3, -1, 0, 2, &[1, 2345, 6789]), n).unwrap();
        let Value::Numeric(ref value) = small else {
            unreachable!()
        };
        assert_eq!(value.to_display_string(), "0.00");
        // A negative value that keeps a digit keeps its sign.
        let kept = decode(&numeric_body(2, 0, 0x4000, 1, &[1, 5678]), n).unwrap();
        assert_eq!(kept, numeric("-1.5"));
    }

    /// Dep-free smoke fuzz: an xorshift stream of random-length, random-content
    /// buffers fed to the decoder for every `TypeOid` must only ever return Ok/Err
    /// — never panic, overflow, or hang. The decoder's twin of the wire codec's
    /// `arbitrary_bytes_never_panic`; catches the whole class of unchecked-slice /
    /// arithmetic-overflow bugs, not just the cases enumerated above.
    #[test]
    fn arbitrary_bytes_never_panic() {
        let oids = [
            TypeOid::Bool,
            TypeOid::Int2,
            TypeOid::Int4,
            TypeOid::Int8,
            TypeOid::Float4,
            TypeOid::Float8,
            TypeOid::Text,
            TypeOid::Varchar,
            TypeOid::Bytea,
            TypeOid::Uuid,
            TypeOid::Timestamp,
            TypeOid::Timestamptz,
            TypeOid::Date,
            TypeOid::Time,
            TypeOid::Jsonb,
            TypeOid::Int4Array,
            TypeOid::TextArray,
            TypeOid::UuidArray,
        ];
        let mut state: u64 = 0x2026_0617_1234_5678;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 48) as usize;
            let buf: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
            for &oid in &oids {
                // The contract is "no panic"; any Result is fine.
                let _ = decode(&buf, oid);
            }
        }
    }
}
