-- `RETURNING` and an `UPDATE`'s `WHERE` and `SET` are evaluated by the same walk.
-- @setup
CREATE TABLE sr (id INT, b BOOL);
INSERT INTO sr VALUES (1, NULL);
-- @test
UPDATE sr SET b = (id = 99 AND 1/(id-1) = 0) WHERE (id = 1 OR 1/(id-1) = 0)
RETURNING b, (id = 1 OR 1/(id-1) = 0) AS r;
