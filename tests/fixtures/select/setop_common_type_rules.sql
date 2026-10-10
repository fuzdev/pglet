-- Set-operation columns unify by the same `select_common_type` rule as `COALESCE` and
-- `CASE` (Postgres uses one algorithm for all of them), so the order-dependence and the
-- float4 preference show up here too: `int4` vs `real` is `real`, and `varchar` before
-- `text` stays `character varying` while the reverse is `text`. pglet's fold used to be
-- a symmetric width promotion, which answered `double precision` for the first and
-- `text` for both spellings of the second.
-- Locked because the shared fold changed; the values render identically
-- either way, so only the declared column types tell the two apart.
-- @setup
CREATE TABLE su (i4 INT, r REAL, t TEXT, v VARCHAR(10));
INSERT INTO su VALUES (1, 2.5, 'aa', 'bb');
-- @test
SELECT x FROM (SELECT i4 AS x FROM su UNION ALL SELECT r FROM su) q ORDER BY x;
