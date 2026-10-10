-- The accept-side control for the byte rule: below `\x80` a byte *is* its code point,
-- so these agreed before the unit moved and must still. `\x4` is `{xehexesc}`'s
-- one-digit form (it takes 1-2) and `\7` / `\77` the short `{xeoctesc}` ones.
-- @test
SELECT E'\x41' = 'A' AS hex_a,
       E'\101' = 'A' AS octal_a,
       E'\x4' = E'\004' AS one_hex_digit,
       E'\7' = E'\x07' AS one_octal_digit,
       E'\77' = '?' AS two_octal_digits,
       E'\x7F' = E'\u007F' AS hex_del,
       E'\177' = E'\u007F' AS octal_del,
       E'\x41\x42' = 'AB' AS two_hex_escapes;
