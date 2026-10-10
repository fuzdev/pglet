-- The sole bare `SELECT *` spelling. A wildcard expands to its input's columns, so the
-- projection list stops indexing the output columns and the gate has to read
-- `Plan::output_columns` instead — which is right here, because every column a `*`
-- produces is a real source column carrying a real type. (A lone bare `*` over a single
-- relation is the one wildcard the planner leaves *unexpanded*, so it is also the only
-- one that reaches the projection list as `Expr::Column { name: "*" }`.)
--
-- **The source is empty, and that is the whole fixture.** Treat the `*` as an ordinary
-- 1:1 projection item and it types as nothing, so the gate declines — and the store
-- finalizer, which does refuse `int4 → bool`, never sees a value to refuse. Measured as
-- that mutation: with a row in `acwcsrc` the statement is `42804` either way and the
-- fixture is green with the bug; with none it is the only red in the corpus.
-- @setup
CREATE TABLE acwc (b BOOL);
CREATE TABLE acwcsrc (n INT);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
INSERT INTO acwc SELECT * FROM acwcsrc;
