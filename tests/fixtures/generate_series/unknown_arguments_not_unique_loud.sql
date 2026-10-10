-- Two string literals fit the integer, bigint and numeric signatures equally: 42725.
-- @expect-error: function generate_series(unknown, unknown) is not unique
-- @test
SELECT * FROM generate_series('1', '3');
