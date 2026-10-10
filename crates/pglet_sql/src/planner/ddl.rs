//! DDL helpers shared with the native parser.
//!
//! CREATE/DROP/ALTER planning is fused into the native parser (`parser::ddl`),
//! which builds the `Plan::Create*`/`Drop*`/`Alter*` nodes directly. The only
//! piece that survives as a standalone planner helper is `is_serial_type`, the
//! auto-increment-type predicate the parser consults while lowering a column's
//! `DEFAULT`.

/// True for the `SERIAL`/`BIGSERIAL`/`SMALLSERIAL` family (auto-increment).
pub fn is_serial_type(type_str: &str) -> bool {
    matches!(
        type_str,
        "serial" | "serial4" | "bigserial" | "serial8" | "smallserial" | "serial2"
    )
}
