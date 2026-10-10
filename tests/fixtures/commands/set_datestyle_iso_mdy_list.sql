-- The accept side of the fixed-parameter gate: `DateStyle` is `ISO, MDY` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. The list spelling, two bare words: `DateStyle` is a list setting, so this is one value and not the `22023 takes only one argument` a scalar setting answers.
-- @test
SET DateStyle = ISO, MDY;
SHOW DateStyle;
