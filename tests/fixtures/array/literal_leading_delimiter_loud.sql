-- A delimiter ahead of the first element is malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{,a}'::bytea[];
