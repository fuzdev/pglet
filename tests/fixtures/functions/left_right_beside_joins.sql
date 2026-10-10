-- A `left(` / `right(` call and `LEFT JOIN` / `RIGHT JOIN` in one statement: the keyword
-- is a call only where an operand starts, and a join keyword everywhere else.
-- @setup
CREATE TABLE ja (id INT, s TEXT);
CREATE TABLE jb (id INT, s TEXT);
INSERT INTO ja VALUES (1, 'apple'), (2, 'banana'), (3, 'cherry');
INSERT INTO jb VALUES (1, 'avocado'), (2, 'blueberry'), (4, 'date');
-- @test
SELECT ja.id, left(ja.s, 1) AS a1, right(jb.s, 2) AS b2
FROM ja LEFT JOIN jb ON left(ja.s, 1) = left(jb.s, 1)
  RIGHT JOIN (SELECT 1 AS k UNION ALL SELECT 2) k ON k.k = ja.id
ORDER BY ja.id;
