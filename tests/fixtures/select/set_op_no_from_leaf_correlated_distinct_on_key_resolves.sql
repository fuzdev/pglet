-- The second discriminator for the pass's `scope_is_complete()` argument, through the
-- other gate: a no-FROM `DISTINCT ON` key inside the leaf may name the enclosing query's
-- relation. The `DISTINCT ON` gate itself declines at expression-subquery depth by design
-- and hands the key to this pass, so closing the pass unconditionally is what would refuse
-- it — measured, `42P01` naming the enclosing relation where both engines answer both
-- rows.
--
-- Its refusing twin over a name that binds to nothing is
-- `select/set_op_no_from_leaf_distinct_on_key_loud.sql`.
-- @setup
CREATE TABLE sop_ct (a INT);
INSERT INTO sop_ct VALUES (7), (8);

-- @test
SELECT a FROM sop_ct WHERE EXISTS (SELECT 1 AS z UNION ALL SELECT (SELECT DISTINCT ON (sop_ct.a) 1 AS q)) ORDER BY a;
