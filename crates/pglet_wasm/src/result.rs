use js_sys::{Array, Object, Reflect, Uint8Array};
use pglet_sql::types::{
    JsonbValue, TypeOid, Value, format_date_text, format_interval_text, format_time_text,
    format_timestamp_text, format_timestamptz_text, format_uuid,
};
use wasm_bindgen::prelude::*;

/// Query result exposed to JS.
#[wasm_bindgen]
#[derive(Debug)]
pub struct QueryResult {
    rows: JsValue,
    row_count: usize,
    command: String,
    fields: JsValue,
}

#[wasm_bindgen]
impl QueryResult {
    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> JsValue {
        self.rows.clone()
    }

    // `#[wasm_bindgen]` exports cannot be `const fn`, so this getter cannot be made const.
    #[allow(clippy::missing_const_for_fn)]
    #[wasm_bindgen(getter, js_name = rowCount)]
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    #[wasm_bindgen(getter)]
    pub fn command(&self) -> String {
        self.command.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn fields(&self) -> JsValue {
        self.fields.clone()
    }
}

/// What a result's rows are made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowShape {
    /// An object of JS values per row, keyed by column name — so a repeated name keeps its
    /// last value, and a column named `__proto__` is an own key (`set_own`). The raw binding's
    /// default; the npm adapter builds its object rows itself, as `pg` builds them.
    Objects,
    /// An array of JS values per row, positional, so every column survives a repeated name.
    Values,
    /// An array of each cell's text form per row, positional.
    Text,
}

impl RowShape {
    /// The shape a `query` / `queryParams` call's two flags ask for; `text` wins.
    #[must_use]
    pub const fn from_flags(text: bool, positional: bool) -> Self {
        if text {
            Self::Text
        } else if positional {
            Self::Values
        } else {
            Self::Objects
        }
    }
}

impl QueryResult {
    /// Converts a pglet QueryResult to a WASM-friendly QueryResult.
    ///
    /// Three shapes of row. By default each row is an object of JS values (`value_to_js`).
    /// `RowShape::Values` makes it an array of the same values, positional, which is what
    /// keeps a repeated column name's every value. `RowShape::Text` makes it an **array of
    /// each cell's text form** — a string, or `null` for a NULL — rendered by the one
    /// column-type-aware encoder the native server's text format uses
    /// (`value_to_text_with_oid`: the `timestamptz` `+00` offset, array literals and all), which
    /// is what a Postgres text-protocol client receives. The npm adapter asks for text when a
    /// caller hands `pg`-style type parsers, which take exactly that text, and for values
    /// otherwise. The shape decides which conversion runs, so a value path does no text work.
    // TODO(perf): one `js_sys` call per cell is most of a wide read's cost — a many-row
    // `SELECT *` spends most of its time here, not in the engine — and a timestamp
    // is formatted to text here only for the adapter to parse it back into a `Date`. Hand JS
    // one payload per result (a buffer decoded once in JS), with the adapter's coercion policy
    // applied in the same pass.
    pub fn from_pglet_result(
        result: pglet_sql::executor::QueryResult,
        shape: RowShape,
    ) -> Result<Self, JsValue> {
        let store = Store::default();
        let rows_array = Array::new();

        for (r, row) in result.rows.iter().enumerate() {
            if shape == RowShape::Values {
                let cells = Array::new();
                for (i, value) in row.iter().enumerate() {
                    let js_value = value_to_js(&store, value, result.columns()[i].type_oid)?;
                    store.append(&cells, i, &js_value)?;
                }
                store.append(&rows_array, r, &cells.into())?;
                continue;
            }
            if shape == RowShape::Text {
                let cells = Array::new();
                for (i, value) in row.iter().enumerate() {
                    let oid = result.columns()[i].type_oid;
                    let cell = pglet_sql::types::value_to_text_with_oid(value, oid)
                        .map_or(JsValue::NULL, |t| JsValue::from_str(&t));
                    store.append(&cells, i, &cell)?;
                }
                store.append(&rows_array, r, &cells.into())?;
                continue;
            }
            let obj = Object::new();
            for (i, value) in row.iter().enumerate() {
                let col = &result.columns()[i];
                let js_value = value_to_js(&store, value, col.type_oid)?;
                store.set_own(&obj, &col.name, &js_value)?;
            }
            store.append(&rows_array, r, &obj.into())?;
        }

        // `pg`'s `FieldDef` keys, in its order, from the descriptor the native server's
        // `RowDescription` is built from — so each reads what `pg` reads off that server — and
        // `format` `'text'`, which is what `pg` reports for every field of a query it sends
        // (measured with and without `types`, on both protocols).
        let fields_array = Array::new();
        for (f, col) in result.columns().iter().enumerate() {
            let field = field_def(
                &col.name,
                f64::from(col.table_oid),
                f64::from(col.column_id),
                f64::from(col.type_oid as u32),
                f64::from(col.type_oid.wire_size()),
                f64::from(col.type_modifier),
            );
            store.append(&fields_array, f, &field)?;
        }

        Ok(Self {
            rows: rows_array.into(),
            row_count: result.rows.len(),
            command: result.command_tag,
            fields: fields_array.into(),
        })
    }
}

/// The JS number a `float4`'s text reads as, not the widened `f32`.
///
/// A client of a Postgres server reads a `float4` as text and parses it — `1.1`, where the
/// widened `f32` is `1.100000023841858` — and node-postgres, postgres.js and the native server's
/// own text leg all hand back that parse. So the value is rendered as pglet's `float4` text
/// (`format_float4_text`, 18.6's `float4out` at the default `extra_float_digits`) and parsed as
/// an `f64`, which is the number node-postgres reads off 18.6 for the same value.
fn float4_as_its_text(f: f32) -> f64 {
    if f.is_finite() {
        pglet_sql::types::format_float4_text(f)
            .parse()
            .unwrap_or_else(|_| f64::from(f))
    } else {
        f64::from(f)
    }
}

#[wasm_bindgen(
    inline_js = "export function field_def(name, tableID, columnID, dataTypeID, \
    dataTypeSize, dataTypeModifier) { return { name, tableID, columnID, dataTypeID, \
    dataTypeSize, dataTypeModifier, format: 'text' }; }"
)]
extern "C" {
    /// A result field's descriptor — `pg`'s `FieldDef` keys, in its order — built as one object **literal**, in one call.
    ///
    /// A literal's properties are *defined* (`CreateDataPropertyOrThrow`), not set, on an object
    /// whose prototype is the realm's own `Object.prototype`: no setter, read-only member or
    /// accessor a page put on `Object.prototype` is reached, and no replaced global `Object`
    /// either, so every key is an own, writable, enumerable, configurable data property holding
    /// pglet's value whatever a page put there — as a row's and a JSONB object's keys are
    /// (`Store::set_own`). The literal itself runs no page code (the glue's decode of the one
    /// string is the glue class `crate::call` sets aside), so the import does not catch. Its key
    /// strings are the snippet's own, never decoded out of this module's memory: a field costs one
    /// call and one string value, where a caught `Reflect.set` per key, each with its key string
    /// decoded, cost several times that (measured; `TRADEOFFS.md`).
    fn field_def(
        name: &str,
        table_id: f64,
        column_id: f64,
        data_type_id: f64,
        data_type_size: f64,
        data_type_modifier: f64,
    ) -> JsValue;
}

#[wasm_bindgen]
extern "C" {
    /// A `Uint8Array`, for the one method bound below.
    #[wasm_bindgen(js_name = Uint8Array)]
    type ByteArray;

    /// `Uint8Array.prototype.set(bytes)`, bound with `catch`: the page owns the prototype, so the
    /// method may be replaced, and an uncaught throw from it would unwind the call.
    #[wasm_bindgen(method, catch, js_name = set)]
    fn set_caught(this: &ByteArray, bytes: &[u8]) -> Result<(), JsValue>;
}

/// Whether `key` is an array index: the canonical decimal spelling of an integer below
/// 2^32 − 1.
fn is_array_index(key: &str) -> bool {
    !key.is_empty()
        && (key == "0" || !key.starts_with('0'))
        && key.bytes().all(|b| b.is_ascii_digit())
        && key.parse::<u64>().is_ok_and(|n| n < 4_294_967_295)
}

/// Converts a pglet Value to a JS value.
///
/// `oid` is the result column's declared type: a `timestamptz` column renders with
/// its `+00` offset (matching the native server's text output), since a bare
/// `Value::Timestamp` can't carry that distinction. An offset-less string would be
/// reparsed in the JS engine's local zone and silently shift the instant.
fn value_to_js(store: &Store, value: &Value, oid: TypeOid) -> Result<JsValue, JsValue> {
    match value {
        Value::Null => Ok(JsValue::NULL),
        Value::Bool(b) => Ok(JsValue::from(*b)),
        Value::Int2(i) => Ok(JsValue::from(*i)),
        Value::Int4(i) => Ok(JsValue::from(*i)),
        Value::Int8(i) => Ok(JsValue::from(*i)),
        Value::Float4(f) => Ok(JsValue::from(float4_as_its_text(*f))),
        Value::Float8(f) => Ok(JsValue::from(*f)),
        Value::Text(s) => Ok(JsValue::from_str(s)),
        // Use the shared pglet_core formatters so WASM renders timestamps/UUIDs
        // byte-identically to the native server's text/binary output (previously
        // diverged: RFC3339 `T`/`Z` vs the canonical space-separated form). A
        // `timestamptz` column carries the `+00` offset.
        Value::Timestamp(ts) if oid == TypeOid::Timestamptz => {
            Ok(JsValue::from_str(&format_timestamptz_text(ts)))
        }
        Value::Timestamp(ts) => Ok(JsValue::from_str(&format_timestamp_text(ts))),
        // A DATE renders as `YYYY-MM-DD` text (the field's `dataTypeID` lets a consumer
        // coerce it to a JS Date if it wants to, like it does for TIMESTAMP).
        Value::Date(d) => Ok(JsValue::from_str(&format_date_text(d))),
        // A TIME renders as `HH:MM:SS[.ffffff]` text (matching the native server and
        // node-postgres, which surfaces `time` as a string).
        Value::Time(t) => Ok(JsValue::from_str(&format_time_text(*t))),
        // NUMERIC surfaces as a string (like node-postgres / `pg`), so arbitrary
        // precision survives the JS number's 53-bit mantissa.
        Value::Numeric(nn) => Ok(JsValue::from_str(&nn.to_display_string())),
        Value::Uuid(bytes) => Ok(JsValue::from_str(&format_uuid(bytes))),
        Value::Bytea(b) => {
            // A `Uint8Array` of the bytes, filled through a caught `Uint8Array.prototype.set`
            // (`ByteArray::set_caught`): js-sys's `copy_from` calls the same method uncaught, so a
            // page that replaced it unwound the call and left the module refusing every call
            // after it; here its throw is the statement's error.
            let len =
                u32::try_from(b.len()).map_err(|_| JsValue::from_str("pglet: bytea too long"))?;
            let arr = Uint8Array::new_with_length(len);
            arr.unchecked_ref::<ByteArray>().set_caught(b)?;
            Ok(arr.into())
        }
        Value::Jsonb(j) => jsonb_to_js(store, j),
        Value::Array(arr) => {
            // Arrays surface to JS as a native array of element values. Element OID
            // is the column's element type where the column is an array, so a
            // `timestamptz[]` keeps its offset even when the value was built from
            // `Value::Timestamp` elements (`ARRAY[timestamptz '…']`), which carry no tz bit.
            let elem_oid = oid.array_element().unwrap_or(arr.element_type);
            let js_arr = Array::new();
            for (i, elem) in arr.elements.iter().enumerate() {
                store.append(&js_arr, i, &value_to_js(store, elem, elem_oid)?)?;
            }
            Ok(js_arr.into())
        }
        // An interval renders as `postgres`-style text (like the native server and
        // node-postgres, which surface `interval` as a string).
        Value::Interval(iv) => Ok(JsValue::from_str(&format_interval_text(iv))),
    }
}

/// Converts a JSONB value to JS.
fn jsonb_to_js(store: &Store, value: &JsonbValue) -> Result<JsValue, JsValue> {
    match value {
        JsonbValue::Null => Ok(JsValue::NULL),
        JsonbValue::Bool(b) => Ok(JsValue::from(*b)),
        // JS numbers are f64, so a jsonb number surfaces as an f64 (the same lossy
        // mapping a JS client gets from `JSON.parse` over the wire); the exact value
        // is preserved in storage and on the text/binary wire, only this JS
        // convenience view narrows.
        JsonbValue::Number(n) => Ok(JsValue::from(n.to_f64())),
        JsonbValue::String(s) => Ok(JsValue::from_str(s)),
        JsonbValue::Array(arr) => {
            let js_arr = Array::new();
            for (i, item) in arr.iter().enumerate() {
                store.append(&js_arr, i, &jsonb_to_js(store, item)?)?;
            }
            Ok(js_arr.into())
        }
        JsonbValue::Object(pairs) => {
            let obj = Object::new();
            for (k, v) in pairs {
                store.set_own(&obj, k, &jsonb_to_js(store, v)?)?;
            }
            Ok(obj.into())
        }
    }
}

/// How result building stores what it builds: every data key is **defined** on its object as an
/// own data property, never set, and every array element — a row, a cell, a field, a JSONB or
/// SQL array's element — becomes an own element, defined or set as below, or the statement fails —
/// short of page code on the prototype chain answering for the element, below;
/// one store, built for one result and dropped with it.
///
/// A set runs the prototype chain, and the page owns the prototypes a result's objects inherit
/// from. Under Node `__proto__` is `Object.prototype`'s accessor, and setting it replaces the
/// object's prototype (`{"__proto__": {"isAdmin": true}}` read back with `isAdmin` inherited);
/// once a host freezes `Object.prototype` (the usual prototype-pollution hardening), a set of a
/// key naming one of its members (`toString`, `constructor`, …) returns `false` and stores
/// nothing; and a read-only or setter-bearing index a page puts on `Array.prototype` would
/// drop an appended element. A define does none of those: a jsonb object reaches JS as
/// `JSON.parse` of its text builds it (what `pg` hands back), and an array holds every element.
///
/// The define is an import that catches, and its `false` — a non-extensible target, which a
/// fresh object is not — is an error, so no store is silently dropped.
///
/// The descriptor it passes has no prototype, so neither its fields' stores nor the define's
/// reads of them (`ToPropertyDescriptor` reads inherited fields too) meet anything a host put on
/// `Object.prototype`. It is built lazily, **once per result** — one per key tripled a jsonb
/// read (measured) — and dropped with the result, so nothing a call builds, a page's replaced
/// `Object` included, sees another call's values, and no value outlives the result it is in.
///
/// An array element is the one store that is **set** when that is sound, because a define on
/// an array index is several times a set's cost and every row, cell and field is one (measured:
/// a 50 000-row integer read doubled). A set of an index the array does not own consults its
/// prototype chain only for that index, so the store checks the chain once per result: when no
/// object on it (`Array.prototype`, `Object.prototype`, or whatever a page chained there) owns
/// an array index, elements are set; when one does, every element is defined. The check is not
/// the whole guarantee, because page code the build itself calls after it — a replaced
/// `TextDecoder.prototype.decode`, which every string of the result passes through, or
/// `Uint8Array.prototype.set`, which a `bytea` value's copy runs — can put an index on the chain
/// mid-build: so a set counts as an element only if it answered `true` **and** moved the array's
/// own `length` past it, which makes a non-throwing index setter that stores nothing the
/// statement's error (measured, both doors). The check reads `length`, not the element: page code
/// on the chain — an accessor index put on `Array.prototype`, before the statement or by those
/// doors, or a proxy spliced into the chain, which passes the check whenever its own keys hold no
/// index — can move the receiver's `length` without storing, which leaves a hole where the element
/// was, or store a value of its own choosing; both are silent (measured), and this store does not
/// detect them.
#[derive(Default)]
struct Store {
    descriptor: std::cell::OnceCell<(Object, JsValue)>,
    define_elements: std::cell::OnceCell<bool>,
}

impl Store {
    /// Give `obj` an own, writable, enumerable, configurable data property `key`, as
    /// `JSON.parse` does (`CreateDataProperty`), whatever the key.
    fn set_own(&self, obj: &Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
        self.define(obj, &JsValue::from_str(key), value)
            .map_err(|e| {
                e.unwrap_or_else(|| {
                    JsValue::from_str(&format!(
                        "pglet: could not define the property {key:?} on a result object"
                    ))
                })
            })
    }

    /// Store `value` as `array`'s element `index`, the array's own length.
    fn append(&self, array: &Array, index: usize, value: &JsValue) -> Result<(), JsValue> {
        let at = u32::try_from(index).map_err(|_| JsValue::from_str("pglet: result too long"))?;
        let refused = || JsValue::from_str("pglet: could not store a result element");
        if self.define_elements(array)? {
            return self
                .define(array, &JsValue::from(at), value)
                .map_err(|e| e.unwrap_or_else(refused));
        }
        // A set that answered `true` stored the element only if the array's own `length` moved
        // past it: page code the build calls after the chain check (a replaced
        // `TextDecoder.prototype.decode` or `Uint8Array.prototype.set`) can put a
        // non-throwing index setter on the chain, which answers `true` and stores nothing.
        // (A setter that moves `length` itself passes this check — the Store doc's residual.)
        if Reflect::set(array, &JsValue::from(at), value)? && array.length() == at + 1 {
            Ok(())
        } else {
            Err(refused())
        }
    }

    /// Whether an object on `array`'s prototype chain owns an array index (a canonical
    /// `0`–`4294967294` key), checked once per result.
    fn define_elements(&self, array: &Array) -> Result<bool, JsValue> {
        if let Some(define) = self.define_elements.get() {
            return Ok(*define);
        }
        let mut define = false;
        let mut proto = Reflect::get_prototype_of(array)?;
        // A cycle is impossible on an ordinary chain; the bound keeps a hostile proxy's from
        // spinning here.
        for _ in 0..64 {
            if proto.is_null() {
                break;
            }
            // An array (`Array.prototype` is one) owns no index exactly when its `length` is
            // `0` — every own index is below it. Any other ordinary object lists its array-index
            // keys first, in ascending order (`OrdinaryOwnPropertyKeys`), so its first key
            // answers for all of them.
            let owns_an_index = if Array::is_array(&proto) {
                Reflect::get(&proto, &JsValue::from_str("length"))?.as_f64() != Some(0.0)
            } else {
                let keys = Reflect::own_keys(&proto)?;
                keys.length() > 0 && keys.get(0).as_string().is_some_and(|k| is_array_index(&k))
            };
            if owns_an_index {
                define = true;
                break;
            }
            proto = Reflect::get_prototype_of(&proto)?;
        }
        if !proto.is_null() && !define {
            define = true;
        }
        let _ = self.define_elements.set(define);
        Ok(define)
    }

    /// Define `key` on `target` as an own data property holding `value`; `Err(None)` for a
    /// define that answered `false`.
    fn define(
        &self,
        target: &Object,
        key: &JsValue,
        value: &JsValue,
    ) -> Result<(), Option<JsValue>> {
        let (descriptor, value_key) = self.descriptor().map_err(Some)?;
        if !Reflect::set(descriptor, value_key, value).map_err(Some)? {
            return Err(None);
        }
        if Reflect::define_property(target, key, descriptor).map_err(Some)? {
            Ok(())
        } else {
            Err(None)
        }
    }

    fn descriptor(&self) -> Result<&(Object, JsValue), JsValue> {
        if let Some(built) = self.descriptor.get() {
            return Ok(built);
        }
        let descriptor = Object::new();
        if !Reflect::set_prototype_of(&descriptor, &JsValue::NULL)? {
            return Err(JsValue::from_str(
                "pglet: could not build a property descriptor",
            ));
        }
        for (name, field) in [
            ("value", JsValue::UNDEFINED),
            ("writable", JsValue::TRUE),
            ("enumerable", JsValue::TRUE),
            ("configurable", JsValue::TRUE),
        ] {
            if !Reflect::set(&descriptor, &JsValue::from_str(name), &field)? {
                return Err(JsValue::from_str(
                    "pglet: could not build a property descriptor",
                ));
            }
        }
        // `set` fails only if the cell was filled meanwhile, and nothing here reenters.
        let _ = self
            .descriptor
            .set((descriptor, JsValue::from_str("value")));
        self.descriptor
            .get()
            .ok_or_else(|| JsValue::from_str("pglet: no property descriptor"))
    }
}

#[cfg(test)]
mod tests {
    use super::float4_as_its_text;

    /// The number a `float4`'s text reads as: `1.1`, not the widened `1.100000023841858`, and
    /// the sign of zero and the infinities kept.
    // exact comparison is the point: the number must be the one the text parses to, bit for bit
    #[allow(clippy::float_cmp)]
    #[test]
    fn a_float4_is_the_number_its_text_reads_as() {
        assert_eq!(float4_as_its_text(1.1), 1.1);
        assert_eq!(float4_as_its_text(0.1), 0.1);
        assert_eq!(float4_as_its_text(-3.4e38), -3.4e38);
        assert!(float4_as_its_text(-0.0).is_sign_negative());
        assert_eq!(float4_as_its_text(f32::INFINITY), f64::INFINITY);
        assert!(float4_as_its_text(f32::NAN).is_nan());
    }
}
