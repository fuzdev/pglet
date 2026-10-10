-- The half of the argument-transform rule that matters most, because Postgres
-- **succeeds** here: `transformCreateStmt` returns before it transforms anything when the
-- relation exists (`parse_utilcmd.c`), so the canonical idempotent-migration idiom skips
-- and the column `DEFAULT`'s constant cast is never coerced at all. pglet raises a set
-- quantifier's verdict — and, with it, 18.6's argument transform — in its **parser**,
-- which runs ahead of every executor-time statement gate, so running that transform for
-- an unquantified call turned this success into `22P02` and failed a client's `prepare()`
-- at `Parse`. `planner::transform_call_args` is gated on a written quantifier for exactly
-- this reason. The column count is the observation that the skip skipped rather than
-- applying the second definition; the `SELECT` is not incidental, since the statement
-- itself returns no rows and a fixture over it alone would pass on an engine that
-- silently replaced the table.
-- @setup
CREATE TABLE ine_cc (b TEXT, extra INT);
INSERT INTO ine_cc VALUES ('kept', 1);
-- @test
CREATE TABLE IF NOT EXISTS ine_cc (b TEXT DEFAULT upper('a'::int));
SELECT count(*)::INT AS n FROM information_schema.columns WHERE table_name = 'ine_cc';
