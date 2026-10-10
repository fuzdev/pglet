// The value-coercion contract: one tested place that maps pglet's wasm-boundary JS
// values to the pg-driver conventions, RECURSING into array elements (the gap every
// hand-rolled adapter had). The raw wasm (`result.rs::value_to_js`) already emits
// int8→BigInt, timestamp/timestamptz/date→string, bytea→Uint8Array, jsonb→parsed,
// uuid→string, and arrays as native JS arrays of those; this layer applies the JS
// *policy* (the two contested knobs + the driver-parity conversions) uniformly to
// scalars and array elements.
//
// Two knobs where pg's convention is contested (§3): int8 (default `bigint` — zero
// info loss, the natural wasm output) and date (default ISO `string` — TZ-proof).
// Everything else follows pg's dominant JS convention (bytea→Buffer, jsonb→object,
// uuid→string, NULL→null).

/**
 * @typedef {'bigint' | 'number' | 'string'} Int8Mode
 * @typedef {'string' | 'date'} DateMode
 * @typedef {{int8?: Int8Mode, date?: DateMode}} CoercionOptions
 */

/**
 * The `infinity` / `-infinity` a date or timestamp column can hold, as node-postgres
 * (`postgres-date`) maps them: the numbers `Infinity` / `-Infinity`, since a `Date`
 * cannot hold either. `undefined` for any other text.
 * @param {string} s
 * @returns {number | undefined}
 */
const infinity_of = (s) => (s === 'infinity' ? Infinity : s === '-infinity' ? -Infinity : undefined);

/**
 * Parse pglet's timestamp text (`YYYY-MM-DD HH:MM:SS[.ffffff][+00]`) to a `Date`,
 * explicitly and always as UTC; `infinity` / `-infinity` are the numbers. pglet has no session zone (≡ Postgres at
 * `TimeZone=UTC`), so a plain `timestamp` (no offset) is UTC too — not the local
 * zone a bare `new Date(text)` would assume. Own the parse rather than lean on V8's
 * lenient non-ISO `Date` handling (not guaranteed by ECMA-262).
 *
 * The fields are set through `setUTCFullYear` / `setUTCHours`, which take the year as written.
 * `Date.UTC` and the multi-argument `Date` constructor read a year of 0 through 99 as 1900
 * through 1999, and pglet's date domain starts at year 0001.
 * @param {string} s
 * @returns {Date | number}
 */
const parse_timestamp_utc = (s) => {
	const inf = infinity_of(s);
	if (inf !== undefined) return inf;
	const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?/.exec(s);
	if (!m) return new Date(NaN);
	const ms = m[7] ? Number((m[7] + '000').slice(0, 3)) : 0;
	const d = new Date(0);
	d.setUTCFullYear(+m[1], +m[2] - 1, +m[3]);
	d.setUTCHours(+m[4], +m[5], +m[6], ms);
	return d;
};

/**
 * The local midnight that starts a calendar day, the year taken as written (see
 * `parse_timestamp_utc`). The day is set at noon and the time of day after it, so no time of
 * day is carried over from another year: node-postgres builds the `Date` in 1900 through 1999
 * for a year of 0 through 99 and then moves the year, which keeps the later year's time of
 * day — `01:00` where the zone skipped that midnight then. A midnight the zone skips in the
 * year itself is the instant the `Date` constructor answers for it.
 * @param {number} year
 * @param {number} month - zero-based
 * @param {number} day
 * @returns {Date}
 */
const local_midnight = (year, month, day) => {
	const d = new Date(2000, 0, 1, 12);
	d.setFullYear(year, month, day);
	d.setHours(0, 0, 0, 0);
	return d;
};

/**
 * A DATE (`YYYY-MM-DD`) under the chosen policy: the ISO string (default, TZ-proof)
 * or a local-midnight `Date` (the pg/node-postgres wart — DATE has no zone but the
 * driver applies one). Opt into the wart with `date: 'date'`, under which `infinity` /
 * `-infinity` are the numbers, as node-postgres answers them.
 * @param {string} s @param {DateMode} mode
 * @returns {string | Date | number}
 */
const coerce_date = (s, mode) => {
	if (mode !== 'date') return s;
	const inf = infinity_of(s);
	if (inf !== undefined) return inf;
	const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
	return m ? local_midnight(+m[1], +m[2] - 1, +m[3]) : new Date(NaN);
};

/** bytea → Node `Buffer` when available (the vitest/jest target), else the raw `Uint8Array`. */
const to_bytea = (u8) => (typeof Buffer !== 'undefined' ? Buffer.from(u8) : u8);

// The type OIDs the policy touches — Postgres's own, which a field's `dataTypeID` carries. Every
// other type is already the right JS shape (bool/int2/int4/float/text/varchar/uuid/jsonb/…).
const INT8 = 20;
const BYTEA = 17;
const TIMESTAMP = 1114;
const TIMESTAMPTZ = 1184;
const DATE = 1082;
/** An array type's OID → its element's, for the arrays of the types above. */
const ELEMENT_OF = new Map([
	[1016, INT8],
	[1001, BYTEA],
	[1115, TIMESTAMP],
	[1185, TIMESTAMPTZ],
	[1182, DATE],
]);

/**
 * The per-value coercer for a scalar type OID, or `null` for an identity type.
 * @param {number} oid @param {Required<CoercionOptions>} opts
 * @returns {((v: any) => any) | null}
 */
const scalar_coercer = (oid, opts) => {
	switch (oid) {
		case INT8:
			// wasm already hands us a BigInt; only convert away from it.
			return opts.int8 === 'number'
				? (v) => Number(v)
				: opts.int8 === 'string'
					? (v) => v.toString()
					: null;
		case TIMESTAMP:
		case TIMESTAMPTZ:
			return (v) => parse_timestamp_utc(v);
		case DATE:
			return opts.date === 'date' ? (v) => coerce_date(v, 'date') : null;
		case BYTEA:
			return (v) => to_bytea(v);
		default:
			return null;
	}
};

/**
 * The coercer for a column's type OID, honoring 1-D arrays of the types above: an array column
 * maps the element coercer over its (already-array) value, preserving NULL elements. `null` =
 * identity (no coercion needed).
 * @param {number} oid @param {Required<CoercionOptions>} opts
 * @returns {((v: any) => any) | null}
 */
const column_coercer = (oid, opts) => {
	const element = ELEMENT_OF.get(oid);
	if (element !== undefined) {
		const elem = scalar_coercer(element, opts);
		if (!elem) return null;
		return (v) => (v == null ? v : v.map((e) => (e == null ? e : elem(e))));
	}
	return scalar_coercer(oid, opts);
};

/**
 * Serialize a JS bound-param value to what pglet's param decoder accepts — the input side,
 * symmetric with `coerce_rows`. Closes the round-trip: int8 comes back as `bigint`, so a
 * `bigint` must go IN too (pglet's decoder can't `JSON.stringify` a BigInt); and `Date` is
 * the natural way to pass a timestamp. So:
 * - `bigint` → its decimal string (decoded to the inferred integer type)
 * - `Date` → pglet's canonical space-separated **UTC** timestamp text (pglet has no session
 *   zone). For a `date` column pass a `'YYYY-MM-DD'` string, not a `Date`.
 * - `Infinity` / `-Infinity` pass through as numbers: the wasm binder spells a non-finite
 *   number `Infinity` / `-Infinity`, which a date or timestamp target reads as its infinity —
 *   so a `±Infinity` read back from one round-trips — and a jsonb target serializes the whole
 *   value as `JSON.stringify` does
 * - arrays recurse (a `bigint[]` / `Date[]` param's elements are converted too)
 * - everything else (number/string/bool/null/a byte view/plain object for jsonb) passes through
 * @param {unknown} v
 */
const to_param = (v) => {
	if (v == null) return v;
	if (typeof v === 'bigint') return v.toString();
	if (v instanceof Date) return v.toISOString().replace('T', ' ').replace('Z', '');
	if (Array.isArray(v)) return v.map(to_param);
	return v;
};

/** Map a bound-param list through {@link to_param}; a nullish list passes through. @param {unknown[]} [params] */
export const coerce_params = (params) => (Array.isArray(params) ? params.map(to_param) : params);

/**
 * Coerce a result's positional rows in place per its `fields`' types, cell `i` by field `i`,
 * applying the policy to scalars AND array elements. NULLs pass through. Positional, so two
 * columns sharing a name are each coerced by their own type. Returns the same `rows`.
 * @param {unknown[][]} rows
 * @param {{name: string, dataTypeID: number}[]} fields
 * @param {CoercionOptions} [options]
 * @returns {unknown[][]}
 */
export const coerce_rows = (rows, fields, options = {}) => {
	const opts = {int8: options.int8 ?? 'bigint', date: options.date ?? 'string'};
	/** @type {[number, (v: any) => any][]} */
	const coercers = [];
	fields.forEach((f, i) => {
		const c = column_coercer(f.dataTypeID, opts);
		if (c) coercers.push([i, c]);
	});
	if (coercers.length === 0) return rows;
	for (const row of rows) {
		for (const [i, coerce] of coercers) {
			const v = row[i];
			if (v != null) row[i] = coerce(v);
		}
	}
	return rows;
};
