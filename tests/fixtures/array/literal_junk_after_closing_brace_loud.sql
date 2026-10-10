-- Only whitespace may follow the closing brace.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a} x'::text[];
