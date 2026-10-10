-- A key swap over two rows holding each other's keys: row `(1, 2)`'s new `k` is `2`,
-- which row `(2, 1)` still holds when it is written — `23505`, uniqueness being checked
-- per row. A control: the in-place reading collides at the same row.
-- @profile-lock
-- @setup
CREATE TABLE sswt (k INT UNIQUE, k2 INT);
INSERT INTO sswt VALUES (1, 2), (2, 1);

-- @expect-error: duplicate key value violates unique constraint (23505)
-- @test
UPDATE sswt SET k = k2, k2 = k;
