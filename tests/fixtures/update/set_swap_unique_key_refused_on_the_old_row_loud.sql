-- A key swap 18.6 refuses: the unique `k2` is checked per row as it is written, and row
-- `(1, 2)`'s new `k2` is the old `1`, which row `(3, 1)` still holds — `23505`. The
-- in-place reading wrote `k2 = 2` and `k2 = 1`, and pglet reported `UPDATE 2`.
-- @profile-lock
-- @setup
CREATE TABLE sswr (k INT, k2 INT UNIQUE);
INSERT INTO sswr VALUES (1, 2), (3, 1);

-- @expect-error: duplicate key value violates unique constraint (23505)
-- @test
UPDATE sswr SET k = k2, k2 = k;
