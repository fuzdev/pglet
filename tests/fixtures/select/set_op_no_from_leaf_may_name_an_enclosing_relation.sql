-- An accept polarity of the leaf's correlated pass, over the shape closest to the
-- second member of the class (a plain correlated reference in the leaf, no key at all):
-- inside an `EXISTS` the leaf may legitimately name the enclosing query's relation, and
-- both engines answer both rows.
--
-- Be precise about what this one guards. It is **not** the discriminator for the pass's
-- `scope_is_complete()` argument: measured under an unconditional close, this statement
-- still answers, because a bare correlated column in the leaf's own projection is not a
-- sub-plan and the pass never reaches it. The two that do discriminate are
-- `select/set_op_no_from_leaf_correlated_scalar_subquery_resolves.sql` and
-- `select/set_op_no_from_leaf_correlated_distinct_on_key_resolves.sql`, each `42P01`
-- naming the enclosing relation under that mutation.
-- @setup
CREATE TABLE sop_ct (a INT);
INSERT INTO sop_ct VALUES (7), (8);

-- @test
SELECT a FROM sop_ct WHERE EXISTS (SELECT 1 AS z UNION ALL SELECT sop_ct.a) ORDER BY a;
