-- @param $1 text \X61
-- @test
-- @expect-error: a bound text converted to bytea is refused as a literal is, 22P02
SELECT $1::text::bytea;
