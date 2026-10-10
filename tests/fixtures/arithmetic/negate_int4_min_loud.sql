-- Negating `int4` MIN fails loud (Postgres `int4um` raises "integer out of
-- range", 22003) rather than wrapping to itself.
-- @test
-- @expect-error: integer out of range
SELECT -('-2147483648'::int4);
