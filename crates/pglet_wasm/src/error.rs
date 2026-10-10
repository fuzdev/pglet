use pglet_sql::{ObjectLocation, Severity};
use wasm_bindgen::prelude::*;

/// Converts a pglet error to a JS error carrying the same structured fields the
/// native server puts on the wire.
///
/// The property **names** are `pg`'s (`pg-protocol`'s `parseErrorMessage` maps
/// `D→detail H→hint P→position s→schema t→table c→column n→constraint`), which is what
/// `pglite` exposes too — so an adapter written against either driver reads the same
/// shape from a `PgletBtree` query. This host reads the same accessors the wire
/// builder (`Error::wire_fields`) is assembled from — `detail`, `hint`,
/// `object_location`, `position` — so nothing here re-decides what an error carries.
///
/// `position` goes out as a **string**, as it does on the wire and in `pg` (which does
/// not parse it).
pub fn pglet_error_to_js(err: &pglet_sql::Error) -> JsValue {
    let error = js_sys::Error::new(&err.to_string());
    set(&error, "severity", Severity::Error.as_str());
    set(&error, "code", err.sqlstate());
    if let Some(detail) = err.detail() {
        set(&error, "detail", &detail);
    }
    if let Some(hint) = err.hint() {
        set(&error, "hint", &hint);
    }
    let ObjectLocation {
        table,
        column,
        constraint,
    } = err.object_location();
    if let Some(table) = &table {
        // `schema` accompanies `table` and never appears alone — the same rule the
        // wire builder applies to `s`.
        set(&error, "schema", pglet_sql::IMPLICIT_SCHEMA);
        set(&error, "table", table);
    }
    if let Some(column) = &column {
        set(&error, "column", column);
    }
    if let Some(constraint) = &constraint {
        set(&error, "constraint", constraint);
    }
    if let Some(position) = err.position() {
        set(&error, "position", &position.to_string());
    }
    error.into()
}

/// Set one own property on the JS error, ignoring a failure the way the previous
/// single-property version did (a frozen object is not worth failing a query over).
fn set(error: &js_sys::Error, key: &str, value: &str) {
    let _ = js_sys::Reflect::set(error, &JsValue::from_str(key), &JsValue::from_str(value));
}
