-- A three-way rotation: `(1, 2, 3)` becomes `(2, 3, 1)`, the last assignment reading the
-- first column's old value.
-- @setup
CREATE TABLE sorot (a INT, b INT, c INT);
INSERT INTO sorot VALUES (1, 2, 3);

-- @test
UPDATE sorot SET a = b, b = c, c = a RETURNING a, b, c;
