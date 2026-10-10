-- The same scanner refusal in an output-column alias. pglet used to answer the row
-- under a column whose descriptor names nothing.
-- @test
-- @expect-error: zero-length delimited identifier
SELECT 1 AS "";
