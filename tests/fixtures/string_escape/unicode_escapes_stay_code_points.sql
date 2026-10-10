-- The accept-side control for the pairing rule: an escape that is neither surrogate
-- half passes through it unchanged, and the `U&'...'` decoder's non-escape arms with it.
-- @test
SELECT E'\u0041' = 'A' AS e_short,
       E'\U00000041' = 'A' AS e_long,
       E'\U0010FFFF' = U&'\+10FFFF' AS e_max,
       U&'\0041' = 'A' AS u_short,
       U&'\+000041' = 'A' AS u_long,
       U&'\\' = '\' AS u_backslash,
       U&'\0041\0042' = 'AB' AS u_two_escapes,
       U&'plain' = 'plain' AS u_plain,
       E'it''s' = 'it''s' AS doubled_quote;
