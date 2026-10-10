-- @test
-- @expect-error: the typed constant refuses `\X` as the cast does, 22P02
SELECT bytea '\X61';
