-- `jsonb || jsonb` is a real Postgres operator (object merge) that pglet does not
-- evaluate, so a stored expression using it is `0A000` (not implemented), not
-- `42883` (no such operator) — and it is rejected at DDL rather than on write.
-- Postgres accepts this DDL, so this is a deliberately pglet-stricter case.
-- @setup
CREATE TABLE jc2 (id INT, a JSONB, b JSONB);
-- @test
-- @expect-error: jsonb || jsonb unimplemented in pglet
CREATE INDEX jc2_i ON jc2 (id) WHERE (a || b) IS NOT NULL;
