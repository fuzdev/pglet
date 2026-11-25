-- A positional argument after a named one is `42601`.
-- @expect-error: positional argument cannot follow named argument
-- @test
SELECT make_interval(hours => 2, 1);
