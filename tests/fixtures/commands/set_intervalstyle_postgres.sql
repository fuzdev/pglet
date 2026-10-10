-- The accept side of the fixed-parameter gate: `IntervalStyle` is `postgres` here and stays so. A spelling 18.6 reads as the value pglet
-- already has is honored, and `SHOW` answers what 18.6 answers after it. A bare word.
-- @test
SET IntervalStyle = postgres;
SHOW IntervalStyle;
