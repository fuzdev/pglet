-- The accept side of the fixed-parameter gate: `standard_conforming_strings` is `on` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. Any Boolean spelling of on.
-- @test
SET standard_conforming_strings TO true;
SHOW standard_conforming_strings;
