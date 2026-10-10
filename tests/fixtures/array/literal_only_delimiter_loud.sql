-- A body holding only a delimiter has no element for it to follow.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{,}'::varchar[];
