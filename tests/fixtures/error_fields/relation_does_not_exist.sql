-- 18.6's noun for a missing relation is `relation`, not `table` — and this is the
-- most-raised message in the engine. `P` is 18.6's alone: pglet's `Stmt`/`Expr` carry
-- no spans, so it omits the field rather than inventing one (`EDGES.md`).
-- @setup
CREATE TABLE t_present (a INT);

-- @test
-- @expect-error: no such relation
-- @expect-message
SELECT * FROM no_such_table_xyz;
