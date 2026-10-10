-- A decimal-adjacent `_` is rejected: `1_.5` stops the numeric literal at `1`,
-- leaving stray tokens the parser can't absorb — a syntax error (Postgres 42601,
-- "trailing junk after numeric literal"; pglet fails loud on the leftover input).
-- @test
-- @expect-error: underscore not between digits
SELECT 1_.5;
