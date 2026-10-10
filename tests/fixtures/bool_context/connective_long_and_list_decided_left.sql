-- A long `AND` list is built as a balanced tree; a subtree whose left operand decides it
-- answers `false` up to its parent, so the list stops at its first `false` operand wherever
-- the division sits after it.
-- @setup
CREATE TABLE sl (a INT);
INSERT INTO sl VALUES (1);
-- @test
SELECT count(*) FROM sl WHERE a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a = 99 AND a/(a-1) = 1 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0 AND a > 0;
