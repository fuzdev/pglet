-- **The write polarity of the computed-key hole.** An unreferenced CTE's body never
-- executes, so the eval-time resolver never ran and the `DELETE` did: `DELETE 2` here
-- against 18.6's `42703` with both rows surviving. Row-count independent for the same
-- reason — seeding the arms changes nothing about a body that is never evaluated.
--
-- The `@expect-error` comparison alone cannot tell a refusal raised *before* the writes
-- from one raised after, so the row survival is asserted separately by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_set_op_computed_sort_key_leaf_binding_to_nothing_does_not_delete`, whose qualified
-- `UPDATE` twin covers the other polarity.
-- @setup
CREATE TABLE sopc_n8 (a INT);
INSERT INTO sopc_n8 VALUES (1), (2);
CREATE TABLE sopca (a INT);
CREATE TABLE sopcb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
WITH c AS (SELECT a FROM sopca UNION SELECT a FROM sopcb ORDER BY bogus + 1)
DELETE FROM sopc_n8;
