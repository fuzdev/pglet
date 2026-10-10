-- A **genuinely recursive** CTE's columns are rebuilt by `recursive_cte_columns` rather
-- than propagated, and the rebuild dropped the enum identity the seed's descriptor had
-- already derived — so the working table's column ordered by its **label bytes**. The
-- ordering half: `happy, ok, sad` was the answer where 18.6 orders `sad, ok, happy`.
--
-- The sort key `z` is deliberately **not** projected and carries **no cast**: a projected
-- `z::text` binds the sort to that text and the fixture passes under either order
-- (`EDGES.md` row 104's trap, and the shape a probe for this defect first took).
-- `enum/cte_order_by_member_position` is the non-recursive control beside it — the same
-- statement under a plain `WITH` was right before and after that repair, which is what
-- makes the recursion the condition rather than the CTE.
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
SELECT k FROM r ORDER BY z, k;
