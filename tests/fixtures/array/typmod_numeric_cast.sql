-- The same, at the other modifier-carrying shape: `numeric(4,1)[]` rounds each
-- element to the declared scale.
-- @test
SELECT '{1.25,2.5}'::numeric(4,1)[] AS n;
