-- `ORDINALITY` is a keyword after `WITH`, so a quoted `"ordinality"` is not it, and the
-- statement is a syntax error at the `WITH`.
-- @expect-error: a quoted ordinality is not the keyword
-- @test
SELECT * FROM unnest(ARRAY[1]) WITH "ordinality";
