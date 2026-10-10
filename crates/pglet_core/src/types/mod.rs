//! Value representation, Postgres type OIDs, text formatting, and index-key
//! encoding.
//!
//! Split into focused submodules, all re-exported flat so `pglet_core::types::*`
//! resolves to the same surface as one file:
//!
//! - `value` — the runtime [`Value`] enum, [`ArrayValue`], [`JsonbValue`], and
//!   [`ColumnDesc`].
//! - `type_oid` — the [`TypeOid`] enum, its conversions, `type_oid_from_name`, and the
//!   type-modifier facts: which types carry a `(…)` ([`type_oid_carries_modifier`]) and
//!   what a carrier admits ([`MAX_CHARACTER_LENGTH`], [`MAX_NUMERIC_PRECISION`],
//!   [`MAX_NUMERIC_SCALE`]).
//! - `text` — the Postgres text-format rendering/parsing functions.
//! - `float_text` / `float_input` — a `float4` / `float8`'s text as 18.6 writes it
//!   ([`format_float8_text`]) and as it reads it ([`parse_float8_text`]).
//! - `binary` — the Postgres binary-format value codec (`value_to_binary` and
//!   its inverse `value_from_binary`).
//! - `json` — the minimal JSON parser (`parse_json` / `parse_jsonb`) into
//!   [`JsonbValue`].
//! - `index_key` — [`Value::encode_index_key`], the order-preserving B-tree key
//!   encoder.
//! - `numeric` / `interval` — the [`PgNumeric`] and [`PgInterval`] value types.
//! - `precision` — the sub-second `(p)` rounding the four temporal types share
//!   ([`round_micros_to_precision`]).
//! - `infinite` — [`Infinite`], the `±infinity` a `date` ([`PgDate`]) or a timestamp
//!   ([`PgTimestamp`]) holds beside its calendar values.
//! - `temporal_domain` — the `date` / `timestamp` domain, `0001-01-01 ..= 9999-12-31`
//!   ([`check_date_domain`], [`check_timestamp_domain`]).

mod binary;
mod float_input;
mod float_text;
mod index_key;
mod infinite;
mod interval;
mod json;
mod numeric;
mod precision;
mod temporal_domain;
mod text;
mod type_oid;
mod value;

pub use binary::*;
pub use float_input::*;
pub use float_text::*;
pub use infinite::*;
pub use interval::*;
pub use json::*;
pub use numeric::*;
pub use precision::*;
pub use temporal_domain::*;
pub use text::*;
pub use type_oid::*;
pub use value::*;
