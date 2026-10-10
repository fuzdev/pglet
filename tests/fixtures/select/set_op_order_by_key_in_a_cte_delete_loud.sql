-- **The write polarity: this class destroyed rows.** An unreferenced CTE's body never
-- executes, so the eval-time resolver — the only judge a set-op's trailing keys had —
-- never ran, and the `DELETE` did: `DELETE 2` here against 18.6's `42703` with both rows
-- surviving.
--
-- The `@expect-error` comparison alone cannot tell a refusal raised *before* the writes
-- from one raised after, so the row survival is asserted separately by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_set_op_sort_key_binding_to_nothing_does_not_delete`. The qualified `UPDATE` twin is
-- `select/set_op_order_by_qualified_key_in_a_cte_update_loud.sql`.
-- @setup
CREATE TABLE sop_n8 (a INT);
INSERT INTO sop_n8 VALUES (1), (2);
CREATE TABLE sopa (a INT);
CREATE TABLE sopb (a INT);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
WITH c AS (SELECT a FROM sopa UNION SELECT a FROM sopb ORDER BY bogus) DELETE FROM sop_n8;
