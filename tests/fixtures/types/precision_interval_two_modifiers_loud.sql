-- The same single-`Iconst` window for `interval`
-- (`SimpleTypename: ConstInterval '(' Iconst ')'`), which used to ride the type-name
-- reader's comma-list pass-through and be accepted.
-- @expect-error: two modifier numbers on interval
-- @test
SELECT '1.5 secs'::interval(3,1);
