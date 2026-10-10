-- The accept side of the statement-level operand type check: a legal aggregate over a
-- grouping that keeps no group answers no row.
-- @setup
CREATE TABLE o (a INT, b INT NOT NULL, t TEXT, j JSONB, bo BOOL);
INSERT INTO o VALUES (1, 10, 'x', '{"k": 1}', true), (2, 0, 'y', '[1]', false);
-- @test
SELECT a, max(t) AS m, sum(b) AS s, count(j) AS c FROM o GROUP BY a HAVING count(*) > 5;
