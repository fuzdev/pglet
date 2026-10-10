-- The accept side of the fixed-parameter gate: `DateStyle` is `ISO, MDY` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. The value `SHOW` reports, as one string.
-- @test
SET DateStyle = 'ISO, MDY';
SHOW DateStyle;
