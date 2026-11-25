-- Quoted, too.
-- @expect-error: no coalesce(a => integer)
-- @test
SELECT "coalesce"(a => 1);
