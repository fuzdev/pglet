-- `SET a = DEFAULT` beside a right-hand side reading `a`: the read takes the old value
-- (1), not the default the assignment to its left resolves (7).
-- @setup
CREATE TABLE sodf (a INT DEFAULT 7, b INT);
INSERT INTO sodf VALUES (1, 0);

-- @test
UPDATE sodf SET a = DEFAULT, b = a RETURNING a, b;
