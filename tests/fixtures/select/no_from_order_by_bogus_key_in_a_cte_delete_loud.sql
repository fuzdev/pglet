-- **The write polarity of the no-FROM sort-key hole, and it is row-count independent.** An
-- unreferenced CTE's body never executes, so the eval-time resolver — the only judge a
-- routed-but-ungated tail leaves — never runs at any array length, and the `DELETE` did:
-- `DELETE 2` here against 18.6's `42703` with both rows surviving.
--
-- The `@expect-error` comparison cannot tell a refusal raised *before* the writes from one
-- raised after, so the row survival is asserted separately by
-- `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_no_from_sort_key_binding_to_nothing_does_not_delete` and its `UPDATE` twin.
-- @setup
CREATE TABLE obnf (a INT);
INSERT INTO obnf VALUES (1), (2);

-- @expect-error: column "zzz" does not exist (42703)
-- @test
WITH c AS (SELECT 1 AS q ORDER BY zzz) DELETE FROM obnf;
