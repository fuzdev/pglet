-- An UPDATE that moves the key it was probed by: the collection finishes before the first
-- write, so `k = 1` moves its row once, and a later probe finds the moved row under its new
-- key. Between them, `k >= 2 AND k < 10` writes 3 → 4 and then 2 → 3, row order.
-- @profile-lock
-- @setup
CREATE TABLE mk (k INT PRIMARY KEY, v TEXT);
INSERT INTO mk VALUES (3, 'c'), (2, 'b'), (1, 'a');
-- @test
UPDATE mk SET k = k + 10 WHERE k = 1;
UPDATE mk SET k = k + 1 WHERE k >= 2 AND k < 10;
UPDATE mk SET v = v || '!' WHERE k = 11 OR k = 4;
UPDATE mk SET v = v || '?' WHERE k = 11 RETURNING k, v;
