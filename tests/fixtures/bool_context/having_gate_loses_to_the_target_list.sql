-- `HAVING` is the opposite: `transformStmt` runs FROM, the target list, the
-- `WHERE` and only then the `HAVING`, so an unbindable projection item wins. A
-- gate placed at pglet's old `HAVING` position — above the projection — would
-- have invented a cross-code divergence here; the walk moved instead.
-- @setup
CREATE TABLE bcord3 (k INT);
-- @expect-error: the target list's 42703, not the HAVING gate's 42804
-- @test
SELECT nosuchcol FROM bcord3 HAVING max(k);
