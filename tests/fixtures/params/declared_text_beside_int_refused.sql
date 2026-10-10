-- A declared `text` `$N` is not an unknown literal: beside an `integer` literal it has no
-- common type, `42804`, where reading it as unknown converts its value and answers.
-- @param-declared $1 text 7
-- @expect-error: COALESCE types text and integer cannot be matched
-- @test
SELECT coalesce($1, 1);
