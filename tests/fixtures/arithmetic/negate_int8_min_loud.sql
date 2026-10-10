-- Negating `int8` MIN fails loud (Postgres `int8um` raises "bigint out of
-- range", 22003) rather than wrapping to itself.
-- @test
-- @expect-error: bigint out of range
SELECT -('-9223372036854775808'::int8);
