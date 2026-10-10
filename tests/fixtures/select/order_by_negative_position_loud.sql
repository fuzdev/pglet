-- `ORDER BY -1` is a **negative position**, not a constant sort key. Postgres's grammar
-- folds unary `-` over a numeric constant into the constant itself (`doNegate`,
-- `gram.y`), so the key is an `A_Const` and `findTargetlistEntrySQL92` reads it as a
-- 1-based ordinal — out of range, `42P10`.
--
-- pglet built an `Expr::Negate` for every unary minus, so `-1` was never a literal, never
-- reached the ordinal's bounds check, and sorted by a constant instead: the statement
-- **ran**. Its `DELETE`/`UPDATE` spellings therefore wrote against an error, which is why
-- the row survival is pinned in `crates/pglet/tests/scope_relation_identity.rs` beside
-- this SQLSTATE. `+1` stays a non-ordinal on both engines
-- (`order_by_unary_plus_not_ordinal.sql`) — the asymmetry is Postgres's own.
-- @setup
CREATE TABLE obnp (x INT, y INT);
INSERT INTO obnp VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
SELECT x FROM obnp ORDER BY -1;
