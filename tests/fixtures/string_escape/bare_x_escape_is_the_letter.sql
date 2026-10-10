-- `{xehexesc}` needs at least one hex digit. A `\x` that fails it is matched by the
-- shorter `{xeescape}` instead, whose `unescape_single_char('x')` returns the letter --
-- so each of these is a string and not an error.
-- @test
SELECT E'\x' AS bare,
       E'\xZZ' AS two_non_digits,
       E'\xG' AS one_non_digit,
       E'a\x' AS trailing,
       E'\x' = 'x' AS bare_is_the_letter;
