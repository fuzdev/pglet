-- An explicit `x::numeric(p,s)` cast whose integer part exceeds `p - s` digits fails
-- loud (Postgres "numeric field overflow", SQLSTATE 22003) — the cast enforces the
-- typmod identically to the column store. `12345.6::numeric(4,1)` needs 5 integer
-- digits but the budget is `4 - 1 = 3`.
-- @test
-- @expect-error: numeric field overflow
SELECT 12345.6::numeric(4,1);
