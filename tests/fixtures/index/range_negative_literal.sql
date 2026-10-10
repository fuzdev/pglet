-- The range half of `eq_negative_literal.sql`: a folded negative bound reaches
-- `IndexRangeScan` where the unfolded `Expr::Negate` reached neither range detector, so
-- the accelerated and stripped profiles take different plans over the same rows. Both
-- bounds are negative so the merge-tighten path sees two comparable negative constants.
-- @profile-lock
-- @setup
CREATE TABLE ixnrg (id INT, k INT);
INSERT INTO ixnrg VALUES (1, -30), (2, -20), (3, -10), (4, 0), (5, 10);
CREATE INDEX ixnrg_k ON ixnrg(k);
-- @test
SELECT id, k FROM ixnrg WHERE k > -30 AND k <= -10 ORDER BY id;
