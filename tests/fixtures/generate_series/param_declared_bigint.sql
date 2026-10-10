-- A parameter declared `bigint` at `Parse` resolves the `bigint` overload.
-- @param-declared $1 int8 3
-- @test
SELECT * FROM generate_series(1, $1);
