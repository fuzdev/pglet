-- The item has one column, so a two-name alias list is 42P10.
-- @expect-error: table has 1 columns available but 2 columns specified
-- @test
SELECT * FROM generate_series(1, 2) AS t(i, j);
