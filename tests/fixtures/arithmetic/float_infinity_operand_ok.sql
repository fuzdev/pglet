-- An already-infinite operand (not a finite→inf overflow) passes through, and a
-- NaN result (inf − inf) is returned, not an error — the overflow/underflow guard
-- fires only on a genuine finite→inf overflow or nonzero→0 underflow (Postgres
-- `float8_pl`/`float8_mi`).
-- @test
SELECT 'Infinity'::float8 + 1 AS a, 'inf'::float8 - 'inf'::float8 AS b;
