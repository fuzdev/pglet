-- @test
-- @expect-error: an array element is refused as a scalar is, 22P02
SELECT '{abcd,"\\X61"}'::bytea[];
