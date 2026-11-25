-- A name for a position already written positionally matches no overload.
-- @expect-error: no make_interval(integer, years => integer)
-- @test
SELECT make_interval(1, years => 2);
