-- Under an alias the column is the alias's name, so the function's name is not a column.
-- @expect-error: column g.generate_series does not exist
-- @test
SELECT g.generate_series FROM generate_series(1, 2) g;
