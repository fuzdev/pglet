-- Qualified, `coalesce` is an ordinary function name, which has no argument of that name
-- (`42883`).
-- @expect-error: no pg_catalog.coalesce(a => integer)
-- @test
SELECT pg_catalog.coalesce(a => 1);
