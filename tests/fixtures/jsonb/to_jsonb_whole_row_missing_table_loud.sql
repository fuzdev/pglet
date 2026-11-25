-- A whole-row reference over a table the catalog lacks is the table's `42P01`.
-- @expect-error: relation does not exist
-- @test
SELECT to_jsonb(x) FROM nosuch x;
