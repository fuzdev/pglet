-- Postgres exposes a sequence as a one-row relation (`last_value`, `log_cnt`,
-- `is_called`); pglet models no such relation and refuses. The refusal has to name that
-- reason: answering 42P01 made one name give two contradictory answers about its own
-- existence, since `CREATE TABLE seqrel` would be 42P07.
--
-- This fixture *documents* the divergence rather than guarding it: Postgres accepts the
-- query, so the harness accepts any pglet error — the old 42P01 passes here too. The
-- three write shapes beside it (`sequence_insert_loud` / `_update_` / `_delete_`) are the
-- real guard, since Postgres errors there and the SQLSTATE has to match.
-- @setup
CREATE SEQUENCE seqrel;
-- @test
SELECT last_value FROM seqrel;
-- @expect-error: pglet-stricter; PG reads a sequence as a relation (42809 here)
