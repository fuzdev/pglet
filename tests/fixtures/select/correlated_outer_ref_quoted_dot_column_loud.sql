-- The read spelling of the refusal pinned on the `DELETE` side in
-- `delete/correlated_outer_ref_quoted_dot_column_loud.sql`, kept because the two build the
-- outer frame at different call sites — this one in `Plan::Project`'s per-row scope, that
-- one in the DML's own `scan_matching_rows` — so a fix confined to one write path would
-- leave this green.
--
-- `q1`'s quoted `"a.b"` column is byte-identical to the `relation.column` addressing a
-- multi-relation outer frame uses; the frame here is single-relation and knows it, so it
-- must not answer for a relation named `a`. 18.6 raises `42P01`; pglet used to return
-- `n = 1, 0` — an entirely ordinary-looking wrong answer.
-- @setup
CREATE TABLE q1 (id INT, "a.b" TEXT);
CREATE TABLE q2 (yv TEXT);
INSERT INTO q1 VALUES (1, 'hit'), (2, 'miss');
INSERT INTO q2 VALUES ('hit');

-- @expect-error: missing FROM-clause entry for table "a" (42P01)
-- @test
SELECT (SELECT count(*) FROM q2 WHERE q2.yv = a.b) AS n, q1.id FROM q1 ORDER BY q1.id;
