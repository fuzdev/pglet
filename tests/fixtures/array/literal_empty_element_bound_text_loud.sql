-- A bound string cast to an array at run time is read by the same reader.
-- @param $1 text {a,,b}
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT ($1::text)::text[];
