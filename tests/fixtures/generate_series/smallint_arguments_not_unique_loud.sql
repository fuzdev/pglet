-- Two `smallint`s promote to `integer`, `bigint` and `numeric` equally: 42725.
-- @expect-error: function generate_series(smallint, smallint) is not unique
-- @test
SELECT * FROM generate_series(1::smallint, 3::smallint);
