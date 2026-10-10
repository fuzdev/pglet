-- A numeric beyond `real`'s finite range fails loud (Postgres float4in "value
-- out of range: overflow", 22003) rather than silently yielding Infinity — the
-- numeric→float4 cast mirrors the float8→float4 overflow guard.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);
-- @expect-error: value out of range: overflow (numeric past real's range)
-- @test
SELECT '1e300'::numeric::float4 FROM t;
