-- The same rebuild as `enum/recursive_cte_order_by_member_position`, at the half that is
-- a **wrong row set** rather than a wrong order: `z > 'ok'` selected the `sad` row by
-- label bytes where 18.6 selects the `happy` one by member position. A guard that only
-- checked "no error" cannot see either half — the broken build answers, it answers
-- wrongly.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'ok'), (2, 'sad'), (3, 'happy');

-- @test
WITH RECURSIVE r(k, z) AS (
  SELECT id, m FROM t
  UNION ALL
  SELECT k, z FROM r WHERE false
)
SELECT k FROM r WHERE z > 'ok' ORDER BY k;
