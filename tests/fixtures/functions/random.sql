-- random() is a float8 in [0, 1), drawn afresh per row: every row's draw is in range.
-- @setup
CREATE TABLE rn (id INT);
INSERT INTO rn VALUES (1), (2), (3), (4), (5), (6), (7), (8);
-- @test
SELECT count(*) AS n FROM (SELECT random() AS r FROM rn) s WHERE r >= 0 AND r < 1;
