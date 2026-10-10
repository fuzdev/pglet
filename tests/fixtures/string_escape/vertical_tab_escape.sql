-- `\v` is the vertical tab, U+000B. The discriminating assertion is the equality
-- against a `\u000B` spelling and it comes first: `length` cannot see this at all, the
-- vertical tab and the letter `v` both being one character.
-- @test
SELECT E'\v' = E'\u000B' AS is_vertical_tab,
       E'\v' = 'v' AS is_the_letter,
       E'\v' = E'\x0B' AS hex_spelling,
       E'\v' = E'\013' AS octal_spelling,
       length(E'\v') AS len;
