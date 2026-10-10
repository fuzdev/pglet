-- The declared type with nothing beside it to unify: `bigint`.
-- @test
SELECT * FROM generate_series(2147483647::bigint, 2147483648);
