-- `to_jsonb` over each scalar kind: numbers and booleans as JSON values, a non-finite
-- float as a string, timestamps in ISO 8601 with a `T` and a `timestamptz`'s offset as
-- `+00:00`, arrays as JSON arrays of their converted elements, the rest as strings of
-- their text output.
-- @test
SELECT to_jsonb(1::int2) AS i2, to_jsonb(5000000000::int8) AS i8, to_jsonb(1.50::numeric) AS n,
       to_jsonb('NaN'::float8) AS nan, to_jsonb(true) AS b, to_jsonb('x"y'::text) AS t,
       to_jsonb('2026-01-02 03:04:05'::timestamptz) AS tz,
       to_jsonb('2026-01-02 03:04:05.123456'::timestamp) AS ts,
       to_jsonb('infinity'::timestamp) AS inf, to_jsonb('2026-01-02'::date) AS d,
       to_jsonb('12:30:00.5'::time) AS tm, to_jsonb('1 day 2 hours'::interval) AS iv,
       to_jsonb('\xdead'::bytea) AS by, to_jsonb('{"b": 1, "a": [2]}'::jsonb) AS j,
       to_jsonb(ARRAY[1, NULL]) AS arr, to_jsonb(ARRAY['2026-01-02'::timestamptz]) AS tzs,
       to_jsonb(NULL::int) AS nul;
