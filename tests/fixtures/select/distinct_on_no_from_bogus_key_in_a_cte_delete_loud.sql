-- **The write polarity: this class destroyed rows.** The no-FROM `DISTINCT ON` site built
-- an empty *and open* scope and never gated its keys, so the only judge left was the
-- eval-time resolver — and an unreferenced CTE's body never executes, so it never ran.
-- The `DELETE` then did: `DELETE 2` here against 18.6's `42703` with both rows surviving.
--
-- **Emptiness is not load-bearing for this polarity**, unlike the read one
-- (`select/distinct_on_no_from_key_binds_to_nothing_loud.sql`), because nothing about this
-- statement depends on the projection's row count: the same `WITH` over
-- `unnest(ARRAY[1,2])`, and over a plain `1 AS q`, each removed both rows too (measured).
-- The empty spelling is kept so this fixture and the read one differ in one thing only.
--
-- The `@expect-error` comparison alone cannot tell a refusal raised *before* the writes
-- from one raised after, so the row survival is asserted separately by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_no_from_distinct_on_key_binding_to_nothing_does_not_delete`. Its `UPDATE` twin is
-- `select/distinct_on_no_from_bogus_key_in_a_cte_update_loud.sql`.
-- @setup
CREATE TABLE dnf_n8 (a INT);
INSERT INTO dnf_n8 VALUES (1), (2);

-- @expect-error: column "bogusdon" does not exist (42703)
-- @test
WITH c AS (SELECT DISTINCT ON (bogusdon) unnest(ARRAY[]::int[]) AS q) DELETE FROM dnf_n8;
