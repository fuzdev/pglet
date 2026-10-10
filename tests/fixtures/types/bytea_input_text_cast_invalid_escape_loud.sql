-- @test
-- @expect-error: a text value holding an invalid escape is 22P02
SELECT ('a' || '\b')::bytea;
