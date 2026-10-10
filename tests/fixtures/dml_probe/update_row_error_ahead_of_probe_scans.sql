-- A conjunct that fails on some rows only, written ahead of the key: pglet evaluates the
-- conjuncts in written order over every row and raises 22012 on the row x = 0, on both
-- build profiles — the write takes no probe that would leave that row out. 18.6 orders the
-- quals by cost and answers `UPDATE 1` (EDGES.md row 172's cost-order residual), so the
-- fixture records its success and requires only that pglet refuse.
-- @profile-lock
-- @expect-error: division by zero (pglet); 18.6 accepts
-- @setup
CREATE TABLE re (id INT PRIMARY KEY, x INT);
INSERT INTO re VALUES (1, 5), (2, 0);
-- @test
UPDATE re SET x = x WHERE 10/x > 0 AND id = 1;
