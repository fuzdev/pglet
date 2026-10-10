-- A join's `ON` condition: the left operand decides it for each pair, so the division is
-- evaluated only for the pair `u.id = 2`, where it does not raise.
-- @setup
CREATE TABLE sj1 (id INT);
CREATE TABLE sj2 (id INT);
INSERT INTO sj1 VALUES (1), (2);
INSERT INTO sj2 VALUES (1), (2);
-- @test
SELECT sj1.id AS a, sj2.id AS b FROM sj1 LEFT JOIN sj2
  ON (sj1.id = 1 OR 1/(sj1.id-1) = 0) AND sj2.id = sj1.id ORDER BY 1, 2;
