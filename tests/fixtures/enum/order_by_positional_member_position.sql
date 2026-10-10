-- `ORDER BY 2` is SQL's **positional** reference: the key's values are read out of the
-- row by index, never resolved as an expression, so the member order has to be looked up
-- by that index too — no expression walk sees this key at all. The sort is inside a
-- derived table under a `LIMIT 1`, which is what makes *which* row survives an assertion
-- about the order rather than about presentation, and keeps the bare enum column out of
-- the projection (`docs/enums.md` § Harness note).
--
-- `sad` is the first declared member and the lexically largest label, so the two rules
-- pick opposite rows.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT
  (SELECT id FROM (SELECT id, m FROM t ORDER BY 2 LIMIT 1) lo) AS lowest,
  (SELECT id FROM (SELECT id, m FROM t ORDER BY 2 DESC LIMIT 1) hi) AS highest;
