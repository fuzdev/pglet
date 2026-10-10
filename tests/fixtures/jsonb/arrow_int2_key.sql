-- The `->` array-index key accepts a 2-byte integer: Postgres registers
-- `(jsonb, integer)` and an `int2` widens into it. pglet used to fall through its
-- `Value` match and return NULL — silently wrong on legal input.
-- @test
SELECT '[10,20,30]'::jsonb -> 1::int2 AS a, '[10,20,30]'::jsonb ->> 2::int2 AS b;
