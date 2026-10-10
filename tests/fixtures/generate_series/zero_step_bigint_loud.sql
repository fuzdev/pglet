-- A zero step is 22023 on the `bigint` overload too.
-- @expect-error: step size cannot equal zero
-- @test
SELECT * FROM generate_series(1::bigint, 3, 0);
