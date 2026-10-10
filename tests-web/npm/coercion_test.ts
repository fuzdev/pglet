/**
 * The value-coercion contract as an exhaustive per-type suite: for representative values
 * (incl. NULL, negatives, the int8 precision boundary, empty arrays, arrays of NULLs,
 * leap-day date, sub-ms timestamp) assert the EXACT coerced JS value. This is the "one
 * tested place" the adapter exists to be — a divergence here
 * is caught once, not re-derived by every consumer. Runs on the assembled pkg-npm build.
 */
import {assert, assertEquals, assertInstanceOf} from '@std/assert';
// deno-lint-ignore no-import-prefix
import {create_pglet_db} from '../../crates/pglet_wasm/pkg-npm/index.js';
// deno-lint-ignore no-import-prefix
import {coerce_rows} from '../../crates/pglet_wasm/pkg-npm/coerce.js';
import type {CoercionOptions} from '../../crates/pglet_wasm/pkg-npm/index.d.ts';

/** SELECT a single scalar expression and return its coerced JS value. */
const val = async (expr: string, coercion?: CoercionOptions): Promise<unknown> => {
	const db = create_pglet_db(coercion ? {coercion} : undefined);
	try {
		const {rows} = await db.query(`SELECT ${expr} AS x`);
		return rows[0].x;
	} finally {
		await db.close();
	}
};

Deno.test('bool / int2 / int4 / float → native (identity)', async () => {
	assertEquals(await val(`true`), true);
	assertEquals(await val(`false`), false);
	assertEquals(await val(`42::int2`), 42);
	assertEquals(await val(`-7::int4`), -7);
	assertEquals(await val(`3.5::float8`), 3.5);
	assertEquals(await val(`-0.25::float4`), -0.25);
});

// A `float4` is the number its text reads as — what node-postgres parses off the wire — not the
// widened `f32`: `1.1`, where the widened value is `1.100000023841858`. Arrays the same.
Deno.test('float4 → the number its text reads as, scalars and array elements', async () => {
	assertEquals(await val('1.1::real'), 1.1);
	assertEquals(await val('0.1::float4'), 0.1);
	assertEquals(await val("ARRAY[1.1, 2.2, NULL]::real[]"), [1.1, 2.2, null]);
	assertEquals(await val("'Infinity'::real"), Infinity);
	assert(Object.is(await val("'-0'::real"), -0));
});

Deno.test('int8 → bigint (default), preserving precision past 2^53', async () => {
	assertEquals(await val(`42::int8`), 42n);
	assertEquals(await val(`-9::int8`), -9n);
	// 2^53+1 is unrepresentable as a JS number; bigint keeps it exact
	assertEquals(await val(`9007199254740993::int8`), 9007199254740993n);
});

Deno.test('int8 knobs — number and string', async () => {
	assertEquals(await val(`42::int8`, {int8: 'number'}), 42);
	assertEquals(await val(`-9::int8`, {int8: 'number'}), -9);
	assertEquals(await val(`42::int8`, {int8: 'string'}), '42');
	assertEquals(await val(`9007199254740993::int8`, {int8: 'string'}), '9007199254740993');
});

Deno.test('text / varchar / uuid → string (identity)', async () => {
	assertEquals(await val(`'hi'::text`), 'hi');
	assertEquals(await val(`''::text`), '');
	assertEquals(await val(`'ab'::varchar`), 'ab');
	assertEquals(await val(`'00000000-0000-0000-0000-000000000001'::uuid`), '00000000-0000-0000-0000-000000000001');
});

Deno.test('bytea → Buffer/Uint8Array with exact bytes; empty', async () => {
	const b = await val(`'\\xdeadbeef'::bytea`);
	assertInstanceOf(b, Uint8Array); // Buffer is a Uint8Array subclass
	assertEquals([...(b as Uint8Array)], [0xde, 0xad, 0xbe, 0xef]);
	assertEquals([...(await val(`''::bytea`) as Uint8Array)], []);
});

Deno.test('timestamp / timestamptz → Date (UTC), sub-ms', async () => {
	const ts = await val(`'2024-06-15 12:30:45.123'::timestamp`);
	assertInstanceOf(ts, Date);
	assertEquals((ts as Date).toISOString(), '2024-06-15T12:30:45.123Z');
	// timestamptz renders `+00`; pglet is always UTC, so the instant is the same
	const tsz = await val(`'2024-06-15 12:30:45.123+00'::timestamptz`);
	assertEquals((tsz as Date).toISOString(), '2024-06-15T12:30:45.123Z');
});

Deno.test('date → ISO string (default) or local-midnight Date (knob); leap day', async () => {
	assertEquals(await val(`'2024-02-29'::date`), '2024-02-29'); // leap day, ISO string default
	const d = await val(`'2024-02-29'::date`, {date: 'date'});
	assertInstanceOf(d, Date);
	assertEquals((d as Date).getFullYear(), 2024);
	assertEquals((d as Date).getMonth(), 1); // February (local midnight)
	assertEquals((d as Date).getDate(), 29);
});

/**
 * Run `run` with the process's local zone set to `tz`, restoring the prior zone after. Fails
 * rather than skips where the runtime did not take the zone, so a leg cannot pass in another one.
 */
const with_tz = async (tz: string, run: () => Promise<void> | void): Promise<void> => {
	const prior = Deno.env.get('TZ');
	const prior_zone = new Intl.DateTimeFormat().resolvedOptions().timeZone;
	Deno.env.set('TZ', tz);
	try {
		assertEquals(new Intl.DateTimeFormat().resolvedOptions().timeZone, tz, `the runtime did not take TZ=${tz}`);
		await run();
	} finally {
		// deleting `TZ` does not re-zone a running Deno process, so set the prior zone back first
		Deno.env.set('TZ', prior ?? prior_zone);
		if (prior === undefined) Deno.env.delete('TZ');
	}
};

/** The fields a `Date` shows in the local zone, as `[year, month, day, hours, minutes, seconds, ms]`. */
const local_fields = (d: Date): number[] => [
	d.getFullYear(),
	d.getMonth() + 1,
	d.getDate(),
	d.getHours(),
	d.getMinutes(),
	d.getSeconds(),
	d.getMilliseconds(),
];

// `Date.UTC` and the multi-argument `Date` constructor read a year of 0 through 99 as 1900
// through 1999; the engine's date domain starts at year 0001. The instant is the same whatever
// the local zone, so each case runs under a zone west of UTC as well.
Deno.test('timestamp / timestamptz → the instant the text names, from year 0001 to 9999, scalars and array elements', async () => {
	const cases = [
		['0001-01-01 00:00:00', '0001-01-01T00:00:00.000Z'],
		['0004-02-29 12:00:00', '0004-02-29T12:00:00.000Z'],
		['0099-12-31 23:59:59.999999', '0099-12-31T23:59:59.999Z'],
		['0100-01-01 00:00:00', '0100-01-01T00:00:00.000Z'],
		['1899-12-31 23:59:59.999', '1899-12-31T23:59:59.999Z'],
		['1900-01-01 00:00:00', '1900-01-01T00:00:00.000Z'],
		['9999-12-31 23:59:59.999999', '9999-12-31T23:59:59.999Z'],
	];
	for (const tz of ['UTC', 'America/New_York']) {
		await with_tz(tz, async () => {
			for (const [text, iso] of cases) {
				const at = `${tz} ${text}`;
				assertEquals((await val(`'${text}+00'::timestamptz`) as Date).toISOString(), iso, at);
				assertEquals((await val(`'${text}'::timestamp`) as Date).toISOString(), iso, at);
				const tsz_arr = await val(`ARRAY['${text}+00'::timestamptz, NULL]`) as (Date | null)[];
				assertEquals([(tsz_arr[0] as Date).toISOString(), tsz_arr[1]], [iso, null], at);
				const ts_arr = await val(`ARRAY['${text}'::timestamp]`) as Date[];
				assertEquals(ts_arr[0].toISOString(), iso, at);
			}
		});
	}
});

// The knob's `Date` is the local midnight that starts the day, the year as written. Before a
// zone's standard time that midnight is at the zone's local mean time, which is the offset
// node-postgres's `Date` carries there too: New York's is 4:56:02 behind UTC, São Paulo's 3:06:28.
Deno.test('date knob → local midnight of the day the text names, from year 0001 to 9999, scalars and array elements', async () => {
	const zones: Record<string, [string, string][]> = {
		'UTC': [
			['0001-01-01', '0001-01-01T00:00:00.000Z'],
			['0004-02-29', '0004-02-29T00:00:00.000Z'],
			['0099-12-31', '0099-12-31T00:00:00.000Z'],
			['0100-01-01', '0100-01-01T00:00:00.000Z'],
			['1899-12-31', '1899-12-31T00:00:00.000Z'],
			['1900-01-01', '1900-01-01T00:00:00.000Z'],
			['9999-12-31', '9999-12-31T00:00:00.000Z'],
		],
		'America/New_York': [
			['0001-01-01', '0001-01-01T04:56:02.000Z'],
			['0004-02-29', '0004-02-29T04:56:02.000Z'],
			['0099-12-31', '0099-12-31T04:56:02.000Z'],
			['0100-01-01', '0100-01-01T04:56:02.000Z'],
			['1899-12-31', '1899-12-31T05:00:00.000Z'],
			['1900-01-01', '1900-01-01T05:00:00.000Z'],
			['9999-12-31', '9999-12-31T05:00:00.000Z'],
		],
		// São Paulo skipped the midnight that starts 1985-11-02, its clocks going from 00:00 to
		// 01:00. Year 0085 has that midnight, and a `Date` built in 1985 and moved to 0085 would
		// keep the 01:00 (`0085-11-02T04:06:28.000Z`, what node-postgres answers).
		'America/Sao_Paulo': [
			['0001-01-01', '0001-01-01T03:06:28.000Z'],
			['0085-11-02', '0085-11-02T03:06:28.000Z'],
			['0099-12-31', '0099-12-31T03:06:28.000Z'],
			['9999-12-31', '9999-12-31T03:00:00.000Z'],
		],
	};
	for (const [tz, cases] of Object.entries(zones)) {
		await with_tz(tz, async () => {
			for (const [text, iso] of cases) {
				const at = `${tz} ${text}`;
				const d = await val(`'${text}'::date`, {date: 'date'}) as Date;
				assertEquals(d.toISOString(), iso, at);
				assertEquals(local_fields(d), [+text.slice(0, 4), +text.slice(5, 7), +text.slice(8), 0, 0, 0, 0], at);
				const arr = await val(`ARRAY['${text}'::date, NULL]`, {date: 'date'}) as (Date | null)[];
				assertEquals([(arr[0] as Date).toISOString(), arr[1]], [iso, null], at);
				// the default mode is the text, whatever the zone
				assertEquals(await val(`'${text}'::date`), text, at);
			}
		});
	}
	// A midnight the zone skipped in the year itself is the instant the `Date` constructor
	// answers for that day: 01:00 local.
	await with_tz('America/Sao_Paulo', async () => {
		const d = await val(`'1985-11-02'::date`, {date: 'date'}) as Date;
		assertEquals(d.toISOString(), '1985-11-02T03:00:00.000Z');
		assertEquals(d.getTime(), new Date(1985, 10, 2).getTime());
	});
});

// Every day of the years the constructors misread and the first one they read as written,
// against a reference that never passes them a year below 100: the Gregorian calendar repeats
// every 400 years, so a day is its twin 400 years on less that many milliseconds. Then every
// day of 1900 through 2037 against the multi-argument `Date` constructor, which reads those
// years as written, in zones whose clocks have skipped a midnight (São Paulo) and a whole day
// (Apia).
Deno.test('every day of years 0001–0100 is its own instant and its own local midnight; a later year is the constructor\'s', async () => {
	const CYCLE_MS = 146097 * 86400000;
	const pad = (n: number, width: number): string => String(n).padStart(width, '0');
	const days_in_month = (y: number, m: number): number => new Date(Date.UTC(y + 400, m + 1, 0)).getUTCDate();
	const fields = [{name: 'ts', dataTypeID: 1184}, {name: 'd', dataTypeID: 1082}];
	const coerced = (y: number, m: number, day: number): [Date, Date] => {
		const text = `${pad(y, 4)}-${pad(m + 1, 2)}-${pad(day, 2)}`;
		const [row] = coerce_rows([[`${text} 13:14:15.678901+00`, text]], fields, {date: 'date'});
		return row as [Date, Date];
	};
	for (const tz of ['UTC', 'America/New_York', 'America/Sao_Paulo', 'Pacific/Apia']) {
		await with_tz(tz, () => {
			let days = 0;
			for (let y = 1; y <= 100; y++) {
				for (let m = 0; m < 12; m++) {
					for (let day = 1; day <= days_in_month(y, m); day++) {
						const [ts, d] = coerced(y, m, day);
						const at = `${tz} ${y}-${m + 1}-${day}`;
						if (ts.getTime() !== Date.UTC(y + 400, m, day, 13, 14, 15, 678) - CYCLE_MS) {
							throw new Error(`${at}: timestamp read as ${ts.toISOString()}`);
						}
						if (local_fields(d).join() !== [y, m + 1, day, 0, 0, 0, 0].join()) {
							throw new Error(`${at}: date read as local ${local_fields(d)} (${d.toISOString()})`);
						}
						days++;
					}
				}
			}
			assertEquals(days, 36524); // a century of the proleptic Gregorian calendar: years 4 through 96 leap, 100 not
			for (let y = 1900; y <= 2037; y++) {
				for (let m = 0; m < 12; m++) {
					for (let day = 1; day <= days_in_month(y, m); day++) {
						const [ts, d] = coerced(y, m, day);
						assertEquals(ts.getTime(), Date.UTC(y, m, day, 13, 14, 15, 678), `${tz} ${y}-${m + 1}-${day}`);
						assertEquals(d.getTime(), new Date(y, m, day).getTime(), `${tz} ${y}-${m + 1}-${day}`);
					}
				}
			}
		});
	}
});

Deno.test('date / timestamp infinity → ±Infinity, as node-postgres maps it', async () => {
	assertEquals(await val(`'infinity'::timestamp`), Infinity);
	assertEquals(await val(`'-infinity'::timestamptz`), -Infinity);
	assertEquals(await val(`'infinity'::date`), 'infinity'); // ISO string default keeps the text
	assertEquals(await val(`'-infinity'::date`, {date: 'date'}), -Infinity);
	assertEquals(await val(`ARRAY['infinity'::timestamptz, NULL]`), [Infinity, null]);
});

Deno.test('jsonb → parsed JS value (identity)', async () => {
	assertEquals(await val(`'{"a":1,"b":[true,null]}'::jsonb`), {a: 1, b: [true, null]});
	assertEquals(await val(`'[1,2,3]'::jsonb`), [1, 2, 3]);
	assertEquals(await val(`'"s"'::jsonb`), 's');
});

Deno.test('NULL of every type → null', async () => {
	for (const t of ['int8', 'int4', 'float8', 'text', 'bytea', 'timestamp', 'date', 'uuid', 'jsonb']) {
		assertEquals(await val(`NULL::${t}`), null, `NULL::${t}`);
	}
});

Deno.test('arrays recurse the per-element policy; empty; NULL elements', async () => {
	// int8[] → bigint[] (the live gap the contract closes)
	assertEquals(await val(`ARRAY[1,2,3]::int8[]`), [1n, 2n, 3n]);
	// same array under the number knob → number[]
	assertEquals(await val(`ARRAY[1,2,3]::int8[]`, {int8: 'number'}), [1, 2, 3]);
	// text[] identity
	assertEquals(await val(`ARRAY['a','b']::text[]`), ['a', 'b']);
	// empty array → []
	assertEquals(await val(`ARRAY[]::int8[]`), []);
	// NULL element preserved as null, not dropped
	assertEquals(await val(`ARRAY[1,NULL,3]::int8[]`), [1n, null, 3n]);
	// timestamp[] elements each → Date
	const tsarr = await val(`ARRAY['2024-06-15 00:00:00','2024-06-16 00:00:00']::timestamp[]`) as Date[];
	assert(Array.isArray(tsarr));
	assertEquals(tsarr.map((d) => d.toISOString()), ['2024-06-15T00:00:00.000Z', '2024-06-16T00:00:00.000Z']);
});
