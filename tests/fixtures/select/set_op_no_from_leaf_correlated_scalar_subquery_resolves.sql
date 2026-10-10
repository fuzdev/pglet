-- The accept polarity one level deeper: the correlated reference sits in a scalar subquery
-- *inside* the no-FROM leaf, which is the exact shape
-- `select/set_op_no_from_leaf_distinct_on_key_loud.sql` refuses when the name binds to
-- nothing. Both engines answer the one matching row, which is what makes the pair a gate
-- rather than a blanket.
-- @setup
CREATE TABLE sop_ct (a INT);
INSERT INTO sop_ct VALUES (7), (8);
CREATE TABLE sop_et (k INT);
INSERT INTO sop_et VALUES (7);

-- @test
SELECT a FROM sop_ct WHERE a IN (SELECT 1 AS z UNION ALL SELECT (SELECT k FROM sop_et WHERE sop_et.k = sop_ct.a)) ORDER BY a;
