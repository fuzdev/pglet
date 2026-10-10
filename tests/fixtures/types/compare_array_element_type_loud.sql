-- `text[] = integer[]` has no operator (`42883`); array equality requires the
-- same array type, not merely the same shape.
-- @setup
CREATE TABLE t (ta TEXT[], ia INT[]);
INSERT INTO t VALUES ('{1,2}', '{1,2}');
-- @test
-- @expect-error: no text[] = integer[] operator
SELECT * FROM t WHERE ta = ia;
