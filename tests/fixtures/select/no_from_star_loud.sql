-- A bare `*` with no FROM clause: `42601`, as with a `WHERE`.
-- @expect-error: SELECT * with no tables
-- @test
SELECT *;
