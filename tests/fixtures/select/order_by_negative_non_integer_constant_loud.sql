-- A folded **non-integer** constant is `42601 non-integer constant in ORDER BY`, not
-- `42P10`: `findTargetlistEntrySQL92` tests `IsA(&aconst->val, Integer)` before it reads
-- the position, and `-1.5` folds to a `Float` const (a `Numeric` literal here). The two
-- SQLSTATEs are the reason the fold has to reach the decimal spelling too — without it
-- `-1.5` is an operator node and the statement sorts by a constant and runs.
-- @setup
CREATE TABLE obnf (x INT, y INT);
INSERT INTO obnf VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: non-integer constant in ORDER BY (42601)
SELECT x FROM obnf ORDER BY -1.5;
