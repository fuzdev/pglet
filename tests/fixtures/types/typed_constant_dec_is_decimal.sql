-- `DEC` is SQL's abbreviation for `DECIMAL`, and 18.6 accepts it wherever `decimal`
-- is accepted. It was in the parser's speculation lead-word set without the
-- type-name reader knowing the spelling, so every `dec` form was loud — a lead word
-- for a type nothing could read. Covers the typed constant, the modifier-bearing
-- form, and the cast the same arm fixes.
-- @test
SELECT dec '1.55' AS bare, dec(4,1) '1.55' AS modifier, 1::dec AS cast;
