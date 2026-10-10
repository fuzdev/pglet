-- A range read whose interval names no key answers no row: an inverted `BETWEEN`.
-- @profile-lock
-- @setup
CREATE TABLE rn (k INT PRIMARY KEY, v TEXT);
INSERT INTO rn VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT k FROM rn WHERE k BETWEEN 3 AND 1;
