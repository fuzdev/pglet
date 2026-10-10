-- The accept side of the fixed-parameter gate: `standard_conforming_strings` is `on` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. A Boolean setting reads `1` as on.
-- @test
SET standard_conforming_strings = 1;
SHOW standard_conforming_strings;
