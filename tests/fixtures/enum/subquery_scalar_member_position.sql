-- A **scalar subquery** carries the enum of its plan's first output column, so a
-- comparison against one orders by member position. The walk holds no plan descriptors
-- of its own, so this is the one arm answered from `Plan::output_columns` — which is
-- also the first reader that half of the descriptor pair has ever had
-- (`docs/enums.md` § The two boundaries).
--
-- `mood`'s declared order is the reverse of its lexical one, so a label-byte answer is
-- the exact negation of each of these.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'happy');

-- @test
SELECT
  (SELECT m FROM u WHERE id = 1) > 'ok' AS gt,
  (SELECT m FROM u WHERE id = 1) < 'ok' AS lt,
  (SELECT m FROM u WHERE id = 1) BETWEEN 'ok' AND 'happy' AS btw,
  (SELECT m FROM u WHERE id = 1) > (SELECT m FROM t WHERE id = 2) AS vs_subquery;
