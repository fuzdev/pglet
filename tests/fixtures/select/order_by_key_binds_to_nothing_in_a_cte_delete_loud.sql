-- **The write polarity, and it is row-count-independent.** An unreferenced CTE's body
-- never executes, so the eval-time resolver — the only judge a top-level sort key had —
-- never ran at any row count: this was `DELETE 2` against 18.6's `42703` with both rows
-- surviving, over an empty source relation *and* over a seeded one (measured both ways).
--
-- Reading the class off its read polarity alone gets that backwards: there
-- (`select/order_by_key_binds_to_nothing_loud.sql`) emptiness *is* load-bearing.
--
-- The `@expect-error` comparison alone cannot tell a refusal raised *before* the delete
-- from one raised after it, so the row survival is asserted separately by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_sort_key_binding_to_nothing_does_not_delete`. The `UPDATE` twin is
-- `select/order_by_key_binds_to_nothing_in_a_cte_update_loud.sql`.
-- @setup
CREATE TABLE sk_src (a INT);
CREATE TABLE sk_tgt (a INT);
INSERT INTO sk_src VALUES (7);
INSERT INTO sk_tgt VALUES (1), (2);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
WITH c AS (SELECT a FROM sk_src ORDER BY bogus) DELETE FROM sk_tgt;
