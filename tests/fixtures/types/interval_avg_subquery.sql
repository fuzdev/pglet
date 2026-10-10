-- `avg(interval)` returns `interval` (Postgres's `interval_avg`), including when
-- the argument names a column of a **derived relation** the catalog can't type.
-- The schema-only inference fell through to `float8` there, so the wire advertised
-- OID 701 while emitting interval bytes — caught only by the binary-format
-- comparison this fixture's default `@test` runs.
-- @setup
CREATE TABLE iv_avg_sub (d INTERVAL);
INSERT INTO iv_avg_sub VALUES ('1 day'), ('2 days');
-- @test
SELECT AVG(d) AS a, SUM(d) AS s FROM (SELECT d FROM iv_avg_sub) x;
