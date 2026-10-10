-- Two adjacent Unicode escapes spelling a UTF-16 surrogate pair combine into one
-- character, and the pairing is cross-form -- `\u` with `\u`, `\U` with `\U`, and
-- either mixture of the two.
-- @test
SELECT E'\uD83D\uDCA9' = U&'\+01F4A9' AS u_then_u,
       E'\U0000D83D\U0000DCA9' = U&'\+01F4A9' AS big_u_then_big_u,
       E'\U0000D83D\uDCA9' = U&'\+01F4A9' AS big_u_then_u,
       E'\uD83D\U0000DCA9' = U&'\+01F4A9' AS u_then_big_u,
       length(E'\uD83D\uDCA9') AS len;
