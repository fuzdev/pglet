-- `pg_column_size` of a constant: 18.6 folds it to a datum built in memory, so a varlena
-- carries a four-byte header (`'\xdeadbeef'::bytea` is 8), a fixed-length value is its
-- `typlen`, and a bare string literal is the C string `unknown` is — its bytes and a
-- terminator (`'abc'` is 4). An array is its header, dimensions and each element under a
-- four-byte header, aligned after each.
-- @test
SELECT pg_column_size('abc') AS unknown_literal,
       pg_column_size('\xdeadbeef'::bytea) AS bytea_c,
       pg_column_size(1::text) AS text_c,
       pg_column_size('hello'::varchar) AS varchar_c,
       pg_column_size(1) AS int4_c,
       pg_column_size(1::int8) AS int8_c,
       pg_column_size(-5) AS negated,
       pg_column_size(12.5) AS numeric_c,
       pg_column_size(1.50::numeric(5,2)) AS numeric_typmod,
       pg_column_size(true) AS bool_c,
       pg_column_size('1 day'::interval) AS interval_c,
       pg_column_size('2024-01-02'::date) AS date_c,
       pg_column_size('{"a":1}'::jsonb) AS jsonb_c,
       pg_column_size(ARRAY[1,2,3]) AS int_array,
       pg_column_size(ARRAY['a','bc']) AS text_array,
       pg_column_size(ARRAY['\xdead'::bytea, '\xbeef'::bytea]) AS bytea_array,
       pg_column_size(NULL::text) AS null_typed,
       pg_column_size(NULL) AS null_unknown;
