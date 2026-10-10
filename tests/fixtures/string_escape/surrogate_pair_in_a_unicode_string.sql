-- The same pairing in the `U&'...'` decoder, whose two spellings are `\XXXX` and
-- `\+XXXXXX`; all four combinations of them pair.
-- @test
SELECT U&'\D83D\DCA9' = U&'\+01F4A9' AS short_then_short,
       U&'\D83D\+00DCA9' = U&'\+01F4A9' AS short_then_long,
       U&'\+00D83D\DCA9' = U&'\+01F4A9' AS long_then_short,
       U&'\+00D83D\+00DCA9' = U&'\+01F4A9' AS long_then_long,
       length(U&'\D83D\DCA9') AS len;
