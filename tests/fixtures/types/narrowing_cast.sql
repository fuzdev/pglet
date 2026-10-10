-- Numeric narrowing casts that stay in range: the result matches Postgres
-- byte-for-byte, including float→int rounding (ties to even, Postgres `rint`).
-- The overflow/out-of-range cases fail loud — see `narrowing_overflow_loud.sql`.
-- @test
SELECT (100000::int8)::int4 AS i8_to_i4,
       (30000::int4)::int2 AS i4_to_i2,
       (2.5::float8)::int4 AS round_even_down,
       (3.5::float8)::int4 AS round_even_up,
       (-2.5::float8)::int4 AS round_even_neg,
       (1.5::float8)::real AS f8_to_f4;
