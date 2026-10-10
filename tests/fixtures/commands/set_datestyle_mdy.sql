-- The accept side of the fixed-parameter gate: `DateStyle` is `ISO, MDY` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. The field order alone: the output format keeps its value.
-- @test
SET DateStyle = 'MDY';
SHOW DateStyle;
