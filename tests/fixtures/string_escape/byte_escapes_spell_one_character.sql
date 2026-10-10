-- A `\xHH` / `\ooo` escape contributes a **byte**, so a run of them that spells one
-- UTF-8 character is one character. Two instruments are needed and neither alone is
-- enough: `length` sees the unit, and the equality against a `\uXXXX` spelling — which
-- names a code point on both engines — sees the character.
-- @test
SELECT length(E'\xC3\xA9') AS hex_pair_len,
       E'\xC3\xA9' = E'\u00E9' AS hex_pair_is_e_acute,
       length(E'\303\251') AS octal_pair_len,
       E'\303\251' = E'\u00E9' AS octal_pair_is_e_acute,
       length(E'\xE2\x82\xAC') AS hex_triple_len,
       E'\xE2\x82\xAC' = E'\u20AC' AS hex_triple_is_euro,
       length(E'\x41\xC3\xA9\x42') AS mixed_len,
       E'\x41\xC3\xA9\x42' = E'\u0041\u00E9\u0042' AS mixed_value;
