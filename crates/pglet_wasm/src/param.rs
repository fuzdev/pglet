//! JS → pglet `Value` parameter decoding for the bound-query wasm path.
//!
//! The inverse of `result::value_to_js`. Given the `TypeOid` that
//! `pglet_sql::planner::infer_param_types` assigned each `$N` from query context,
//! each JS argument is mapped to a `Value` — reusing `pglet_sql`'s tested
//! `coerce_value` (text → typed) so uuid / jsonb / timestamp / array parsing is
//! shared with the differential harness's `-- @param` path rather than re-rolled.

use js_sys::{Array, JSON, Reflect};
use pglet_sql::executor::coerce_value;
use pglet_sql::host;
use pglet_sql::types::{ArrayValue, TypeOid, Value, format_bytea_hex};
use wasm_bindgen::prelude::*;

use crate::error::pglet_error_to_js;

/// Decode a JS argument list into bound `$N` values against their inferred types.
///
/// The binding callback the wasm bindings hand to `pglet::Database::execute_bound`:
/// the facade parses the statement and infers each parameter's `oids`, then this
/// decodes the matching JS argument with [`js_to_value`]. A missing `oid` (more
/// args than the plan pins) decodes by the JS runtime type. Returns one
/// `Option<Value>` per argument (`None` = SQL `NULL`); an empty `params` yields an
/// empty `Vec`, so the statement runs with no bound parameters.
pub fn bind_js_params(
    params: &[JsValue],
    oids: &[Option<TypeOid>],
) -> Result<Vec<Option<Value>>, JsValue> {
    let mut bound = Vec::with_capacity(params.len());
    for (i, raw) in params.iter().enumerate() {
        let oid = oids.get(i).copied().flatten();
        refuse_text_outside_the_server_encoding(raw, oid)?;
        bound.push(js_to_value(raw, oid)?);
    }
    Ok(bound)
}

/// Refuse a parameter whose text the server encoding does not hold, before any of it
/// is decoded against its type.
///
/// 18.6's `exec_bind_message` runs `pg_client_to_server` over a text-format
/// parameter's bytes ahead of that parameter's input function, whatever its type, so
/// `SELECT $1::int4` bound to `1<NUL>` is `22021` there, and so is `$1::int4[]` bound
/// to `{x,1<NUL>}`, whose first element `int4`'s input would refuse (both measured over
/// the wire). The loop above checks one parameter and then decodes it, which is the
/// same per-parameter order.
///
/// A JS string reaches Rust as a `String`, so the only byte the scan can refuse here is
/// a NUL. The JS strings decoded as text without being re-spelled first are the
/// parameter itself and, for an array target, the array's direct string items
/// (`decode_array`); any other value becomes text, where it does, through a number's
/// or a boolean's spelling or `JSON.stringify`, which writes a NUL as an escape. So a
/// JS array bound to a jsonb target is serialized rather than scanned, and a NUL in it
/// reaches jsonb's own `22P05`, which is also 18.6's answer to the escaped spelling.
fn refuse_text_outside_the_server_encoding(
    js: &JsValue,
    oid: Option<TypeOid>,
) -> Result<(), JsValue> {
    if let Some(text) = js.as_string() {
        return refuse_text(&text);
    }
    if oid.is_some_and(TypeOid::is_array)
        && let Some(items) = array_items(js)?
    {
        for item in items {
            if let Some(text) = item.as_string() {
                refuse_text(&text)?;
            }
        }
    }
    Ok(())
}

// A parameter is the caller's own JS, and decoding it runs that JS: an array's index
// and `length` getters (a `Proxy`'s traps), a byte view's `buffer`, `byteOffset` and
// `byteLength`, a revoked `Proxy` that `Array.isArray` throws on, a `toJSON`. An exception
// from any of them that crossed this module's frames would unwind them — no destructor
// run, the shadow-stack pointer left where they had it — and poison the module for every
// later call (`crate::call`). So every such read goes through an import that **catches**
// (`Reflect.get`, `Reflect.apply`, `Reflect.construct`, `JSON.stringify`), marked as a
// call out to the host (`pglet_sql::host::call`), and the caller's exception comes back as
// the binding's error — a copy out of a `Uint8Array` this module constructed included
// (`ByteSource`), since it runs `Uint8Array.prototype.set`. What
// stays uncaught is `typeof` (`as_string`, `as_f64`, `as_bool`, `is_object`, `is_null`), which
// reads no caller JS, and the glue's copy of a string `as_string` answers into this module's
// memory, which runs `TextEncoder.prototype.encodeInto` for a non-ASCII one — a page that
// replaced it unwinds the call (measured), the glue class `crate::call` sets aside.

#[wasm_bindgen]
extern "C" {
    /// A `Uint8Array` this module constructed, for the one method bound below.
    #[wasm_bindgen(js_name = Uint8Array)]
    type ByteSource;

    /// The array's bytes, into this module's memory: `valueOf()` answers the array itself,
    /// which the binding copies in with `Uint8Array.prototype.set` — inside the binding's
    /// `catch`, so a throw from a page's replaced `valueOf` (`Object.prototype`'s, unless a page
    /// put one nearer) or `set` is this call's error; one that answers other bytes is bound as
    /// they are, page code lying about the value.
    #[wasm_bindgen(method, catch, js_name = valueOf)]
    fn caught_bytes(this: &ByteSource) -> Result<Vec<u8>, JsValue>;
}

/// A global of the host by name, read through an import that catches.
fn global(name: &str) -> Result<JsValue, JsValue> {
    host::call(|| Reflect::get(&js_sys::global(), &JsValue::from_str(name)))
}

/// Whether the host's `owner.method(js)` answers truthy for the object `js` — a
/// primitive is `false` unasked — read and applied through imports that catch.
fn host_says(owner: &str, method: &str, js: &JsValue) -> Result<bool, JsValue> {
    if !js.is_object() {
        return Ok(false);
    }
    let owner = global(owner)?;
    let method = host::call(|| Reflect::get(&owner, &JsValue::from_str(method)))?;
    // A page that replaced the method with a non-function is the caught `TypeError`.
    host::call(|| Reflect::apply(method.unchecked_ref(), &owner, &Array::of1(js)))
        .map(|answer| answer.is_truthy())
}

/// `Array.isArray(js)`, caught — it throws for a revoked `Proxy`.
fn is_array(js: &JsValue) -> Result<bool, JsValue> {
    host_says("Array", "isArray", js)
}

/// `js`'s items when it is a JS array, read as `Array`'s own iteration reads them —
/// `length` once, then each index — through imports that catch.
fn array_items(js: &JsValue) -> Result<Option<Vec<JsValue>>, JsValue> {
    if !is_array(js)? {
        return Ok(None);
    }
    let length = host::call(|| Reflect::get(js, &JsValue::from_str("length")))?
        .as_f64()
        .unwrap_or(0.0);
    let length = length as u32;
    let mut items = Vec::with_capacity(length as usize);
    for i in 0..length {
        items.push(host::call(|| Reflect::get_u32(js, i))?);
    }
    Ok(Some(items))
}

/// The bytes of `js` when it is a byte view — `ArrayBuffer.isView`: a typed array of any
/// element type, a `DataView`, a Node `Buffer` — which are the bytes of its **window**,
/// `byteLength` of them from `byteOffset` into its buffer, whatever its element type.
///
/// That is the value node-postgres sends for one (`prepareValue`'s `ArrayBuffer.isView`
/// arm reads the same three properties). `isView` reads an internal slot and runs no
/// caller JS; the three properties are read through an import that catches, and the
/// window is copied through a `Uint8Array` this module constructs over it, caught. An
/// `ArrayBuffer` or a `SharedArrayBuffer` is not a view, and neither is a `Proxy` of a view.
fn view_bytes(js: &JsValue) -> Result<Option<Vec<u8>>, JsValue> {
    if !host_says("ArrayBuffer", "isView", js)? {
        return Ok(None);
    }
    let constructor = global("Uint8Array")?;
    let part = |name: &str| host::call(|| Reflect::get(js, &JsValue::from_str(name)));
    let window = Array::of3(&part("buffer")?, &part("byteOffset")?, &part("byteLength")?);
    // A replaced `Uint8Array` that is not a constructor is the caught `TypeError`.
    let copy = host::call(|| Reflect::construct(constructor.unchecked_ref(), &window))?;
    // Out of the copy through an import that catches: js-sys's `to_vec` runs the page's
    // `Uint8Array.prototype.set` uncaught, and a replaced one unwound the call.
    host::call(|| copy.unchecked_ref::<ByteSource>().caught_bytes()).map(Some)
}

/// A byte view's bytes as the characters a text target reads: `22021` for bytes the
/// server encoding does not hold.
fn view_text(bytes: Vec<u8>) -> Result<String, JsValue> {
    pglet_core::encoding::text_from_utf8(bytes).map_err(|e| pglet_error_to_js(&e))
}

/// The refusal of a JS value whose kind the target's type does not take: `22023`.
fn refuse_kind(message: &str) -> JsValue {
    pglet_error_to_js(&pglet_sql::Error::InvalidParameterValue {
        message: message.to_owned(),
        hint: None,
    })
}

/// `22021` naming the bytes from the first failure, through `pglet_core::encoding`'s
/// scan.
fn refuse_text(text: &str) -> Result<(), JsValue> {
    pglet_core::encoding::utf8_failure_bytes(text.as_bytes()).map_or(Ok(()), |bytes| {
        Err(pglet_error_to_js(&pglet_sql::Error::InvalidEncoding {
            bytes,
        }))
    })
}

/// Decode one JS argument into an optional bound `Value`.
///
/// JS `null` / `undefined` → SQL `NULL` (`Ok(None)`). `oid` is the inferred
/// parameter type; `None` means inference couldn't pin it (rare — the spine's
/// `$N::type` casts make it concrete), in which case the JS runtime type decides.
pub fn js_to_value(js: &JsValue, oid: Option<TypeOid>) -> Result<Option<Value>, JsValue> {
    if js.is_null() || js.is_undefined() {
        return Ok(None);
    }
    let value = match oid {
        Some(t) if t.is_array() => decode_array(js, t)?,
        Some(t) => decode_scalar(js, t)?,
        None => infer_scalar(js)?,
    };
    Ok(Some(value))
}

/// Decode a JS value into a scalar `Value` of the known target `oid`.
///
/// What is bound is decided by the value's **kind** before any reader sees text made
/// from it — `bytea`'s reader takes text of any shape, so it cannot be left to refuse a
/// spelling that is not the value's own:
///
/// - a byte view ([`view_bytes`]) is its bytes: a `bytea` target's value, and a `text` or
///   `varchar` target's characters (`22021` for bytes the server encoding does not
///   hold). node-postgres sends one in the binary format, which any other type reads as
///   its own binary layout, so any other target refuses it;
/// - a `jsonb` target takes any other JS value: a string is JSON text already, anything
///   else is serialized to JSON text;
/// - a `bytea` target takes a string, a number or a boolean as its text, and a JS array
///   as the literal node-postgres sends ([`driver_text`]) — each what 18.6 stores for the
///   same bind. Any other object is refused: node-postgres sends its JSON, and the bytes
///   of `{}` are not what an `ArrayBuffer`, or a `Buffer` that went through JSON, was
///   bound to store;
/// - a `text` or `varchar` target is handed [`driver_text`], and a remaining target
///   [`js_to_text`]'s spelling, for its input function to refuse where the value is not
///   one of its own.
fn decode_scalar(js: &JsValue, oid: TypeOid) -> Result<Value, JsValue> {
    let text = if let Some(bytes) = view_bytes(js)? {
        match oid {
            TypeOid::Bytea => return Ok(Value::Bytea(bytes)),
            TypeOid::Text | TypeOid::Varchar => view_text(bytes)?,
            _ => {
                return Err(refuse_kind(
                    "a byte view is bound to a bytea, text or varchar parameter only",
                ));
            }
        }
    } else if oid == TypeOid::Jsonb {
        js.as_string().map_or_else(|| stringify_json(js), Ok)?
    } else if matches!(oid, TypeOid::Text | TypeOid::Varchar | TypeOid::Bytea) {
        if oid == TypeOid::Bytea && js.is_object() && !is_array(js)? {
            return Err(refuse_kind(
                "an object bound to a bytea parameter must be a byte view or an array",
            ));
        }
        driver_text(js)?
    } else {
        js_to_text(js)?
    };
    coerce_value(Value::Text(text), oid).map_err(|e| pglet_error_to_js(&e))
}

/// The text node-postgres sends for `js`, where `js` is not a byte view, for a target
/// whose reader takes text of any shape (`text`, `varchar`, and `bytea`'s escape format).
///
/// node-postgres serializes a value without knowing the parameter's type: a JS array
/// goes out as an array literal with each string, number and boolean element quoted —
/// `{"1","2"}` — a nested array nested, a null `NULL`, a byte view element `\\x` and its
/// bytes' hex. Measured against 18.6. An array target has a reader of its own
/// ([`decode_array`]: a byte view or a nested array that is an array target's element is
/// settled by that function's element rule before this one is asked).
fn driver_text(js: &JsValue) -> Result<String, JsValue> {
    let Some(items) = array_items(js)? else {
        return js_to_text(js);
    };
    let mut text = String::from("{");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        if item.is_null() || item.is_undefined() {
            text.push_str("NULL");
        } else if is_array(item)? {
            text.push_str(&driver_text(item)?);
        } else if let Some(bytes) = view_bytes(item)? {
            text.push('\\');
            text.push_str(&format_bytea_hex(&bytes));
        } else {
            text.push('"');
            for c in js_to_text(item)?.chars() {
                if c == '\\' || c == '"' {
                    text.push('\\');
                }
                text.push(c);
            }
            text.push('"');
        }
    }
    text.push('}');
    refuse_text(&text)?;
    Ok(text)
}

/// Decode a JS value into a `Value::Array` of the known array `oid`.
///
/// An element is not a scalar parameter: node-postgres writes the whole array as one
/// literal, by an element rule of its own. Where that rule and the scalar reading of
/// the element type part, the element is read by the element rule or refused:
///
/// - a byte view element ([`view_bytes`]) is `\x` and the bytes' hex
///   ([`format_bytea_hex`]) for the element type's input to read — a `bytea` element is
///   those bytes, a `text` one the spelling itself, an `int4` or a `jsonb` one `22P02`,
///   each as 18.6 answers node-postgres's literal;
/// - a nested array is a second dimension there, which a `Value::Array` does not
///   have, so it is refused whatever the element type — before the element type's
///   reader is asked, since a reader that takes the nested array's JSON spelling
///   (`bytea`, `jsonb`, the timestamp types) would store it as one element.
fn decode_array(js: &JsValue, oid: TypeOid) -> Result<Value, JsValue> {
    let element_type = oid
        .array_element()
        .ok_or_else(|| JsValue::from_str("array param target is not an array type"))?;
    if let Some(items) = array_items(js)? {
        let mut elements = Vec::with_capacity(items.len());
        for item in items {
            let element = if element_type != TypeOid::Bytea
                && let Some(bytes) = view_bytes(&item)?
            {
                coerce_value(Value::Text(format_bytea_hex(&bytes)), element_type)
                    .map_err(|e| pglet_error_to_js(&e))?
            } else if is_array(&item)? {
                return Err(pglet_error_to_js(&pglet_sql::Error::NotImplemented(
                    "multidimensional arrays are not supported".to_owned(),
                )));
            } else {
                // A null element is `Value::Null` in the array, not a dropped slot.
                js_to_value(&item, Some(element_type))?.unwrap_or(Value::Null)
            };
            elements.push(element);
        }
        return Ok(Value::Array(ArrayValue {
            element_type,
            elements,
        }));
    }
    // Fall back to a Postgres array text literal (`{a,b}`) if one was passed.
    if let Some(s) = js.as_string() {
        return coerce_value(Value::Text(s), oid).map_err(|e| pglet_error_to_js(&e));
    }
    Err(JsValue::from_str(
        "array param must be a JS array or a Postgres array literal string",
    ))
}

/// Best-effort `Value` from a JS value when the type couldn't be inferred.
fn infer_scalar(js: &JsValue) -> Result<Value, JsValue> {
    if let Some(b) = js.as_bool() {
        return Ok(Value::Bool(b));
    }
    if let Some(n) = js.as_f64() {
        return Ok(integral_or_float(n));
    }
    if let Some(s) = js.as_string() {
        return Ok(Value::Text(s));
    }
    // An untyped parameter is `text` on 18.6, and a byte view its characters there.
    if let Some(bytes) = view_bytes(js)? {
        return view_text(bytes).map(Value::Text);
    }
    Err(JsValue::from_str(
        "cannot infer a pglet type for an untyped JS parameter",
    ))
}

/// Render a JS scalar as the text `coerce_value` parses.
fn js_to_text(js: &JsValue) -> Result<String, JsValue> {
    if let Some(s) = js.as_string() {
        return Ok(s);
    }
    if let Some(b) = js.as_bool() {
        return Ok(if b { "true" } else { "false" }.to_owned());
    }
    if let Some(n) = js.as_f64() {
        return Ok(format_number(n));
    }
    // Objects/arrays for a non-array, non-jsonb target: serialize and let
    // `coerce_value` reject the mismatch loudly.
    stringify_json(js)
}

/// `JSON.stringify`, surfaced as a loud error rather than a silent `undefined`.
fn stringify_json(js: &JsValue) -> Result<String, JsValue> {
    // Caught, and a call out to the host: `toJSON` is the caller's code.
    let json = host::call(|| JSON::stringify(js))
        .map_err(|_| JsValue::from_str("cannot JSON-serialize parameter"))?;
    json.as_string()
        .ok_or_else(|| JsValue::from_str("cannot JSON-serialize parameter"))
}

/// Whether `n` is a whole number an `i64` holds exactly — the cast saturates outside
/// that range, which would bind a different number.
fn is_int8(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() < 9_223_372_036_854_775_808.0
}

/// A whole-valued JS number an `int8` holds is one; any other is a `float8`.
fn integral_or_float(n: f64) -> Value {
    if is_int8(n) {
        Value::Int8(n as i64)
    } else {
        Value::Float8(n)
    }
}

/// Text form of a JS number for `coerce_value`: JS's `String` spelling, which is what
/// node-postgres sends, except a whole number an `int8` holds, which is spelled exactly
/// (JS rounds one past 2^53 to seventeen digits).
///
/// A whole number has no fractional part (`3`, not `3.0`), so an integer target reads it;
/// a magnitude from `1e21` up or below `1e-6` is in exponent form with a signed exponent
/// (`1e+21`, `1.5e-7`); the non-finite ones are `Infinity`, `-Infinity`, `NaN`. Rust's own
/// spelling of an infinity is `inf`, and the readers part over it: the `float4` / `float8`
/// readers take `inf` as their infinity, as 18.6's do, while the `date` and timestamp
/// readers refuse it and a `text` target keeps the three letters. `Infinity` is read by
/// the float readers and, case-insensitively, by the `date` and timestamp readers as
/// their infinity.
fn format_number(n: f64) -> String {
    if is_int8(n) {
        return (n as i64).to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if n.abs() >= 1e21 || n.abs() < 1e-6 {
        let text = format!("{n:e}");
        return match text.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                format!("{mantissa}e+{exponent}")
            }
            _ => text,
        };
    }
    n.to_string()
}
