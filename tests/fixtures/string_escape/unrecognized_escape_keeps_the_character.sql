-- The rule `\v` used to reach before it had an arm of its own, and it still holds for
-- an escape no rule matches: the character survives and the backslash is dropped.
-- `\8` and `\9` sit outside `{xeoctesc}`'s `[0-7]`, and a backslash before a
-- multi-byte character keeps that character's bytes whole.
-- @test
SELECT E'\z' = 'z' AS letter,
       E'\8' = '8' AS eight,
       E'\9' = '9' AS nine,
       E'\.' = '.' AS dot,
       E'\%' = '%' AS percent,
       E'\"' = '"' AS double_quote,
       E'\é' = E'\u00E9' AS non_ascii,
       E'\💩' = U&'\+01F4A9' AS astral;
