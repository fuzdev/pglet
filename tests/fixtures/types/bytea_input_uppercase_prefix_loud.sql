-- @test
-- @expect-error: `\X` is no hex prefix: an invalid escape, 22P02
SELECT '\X6162'::bytea;
