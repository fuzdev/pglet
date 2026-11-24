-- `WITH ORDINALITY` stands before the alias, never after it.
-- @expect-error: WITH ORDINALITY after the alias
-- @test
SELECT * FROM unnest(ARRAY[1]) AS h(x) WITH ORDINALITY;
