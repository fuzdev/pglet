-- **The write polarity: this class destroyed every row.** An unreferenced CTE's body never
-- executes, so no eval-time resolver ever saw the key, and with the leaf's correlated pass
-- missing nothing else did either: `DELETE 2` here against 18.6's `42703` with both rows
-- surviving. Row-count independent for the same reason.
--
-- Row survival is asserted separately by `crates/pglet/tests/scope_relation_identity.rs`'s
-- `a_set_op_no_from_leaf_bogus_key_does_not_delete` — an `@expect-error` fixture is green
-- for a refusal raised *after* the writes.
-- @setup
CREATE TABLE sop_n10 (a INT);
INSERT INTO sop_n10 VALUES (1), (2);

-- @expect-error: column "bogusdon" does not exist (42703)
-- @test
WITH c AS (SELECT 1 AS z UNION ALL SELECT (SELECT DISTINCT ON (bogusdon) 1 AS q)) DELETE FROM sop_n10;
