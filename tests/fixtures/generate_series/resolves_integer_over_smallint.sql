-- A `smallint` beside an `integer` resolves the `integer` overload.
-- @test
SELECT * FROM generate_series(1::smallint, 3);
