-- The branch-unification half of the same `RETURNING` gap, on INSERT: a set with no
-- common type was accepted and the projection declared whichever type the first branch
-- happened to carry.
-- @setup
CREATE TABLE irn (id INT, i4 INT4, tx TEXT);
-- @expect-error: COALESCE branches have no common type in RETURNING (42804)
-- @test
INSERT INTO irn VALUES (1, 10, 'a') RETURNING COALESCE(i4, tx);
