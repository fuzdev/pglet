-- Negating `int2` MIN fails loud (Postgres `int2um` raises "smallint out of
-- range", 22003) rather than wrapping to itself — `-(-32768::int2)` has no int2
-- counterpart.
-- @test
-- @expect-error: smallint out of range
SELECT -('-32768'::int2);
