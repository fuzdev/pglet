-- The accept side of the fixed-parameter gate: `DateStyle` is `ISO, MDY` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. An empty list changes nothing on 18.6.
-- @test
SET DateStyle = '';
SHOW DateStyle;
