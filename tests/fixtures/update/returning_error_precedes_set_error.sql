-- The other half of `update/where_error_precedes_set_error.sql`, and the reason that
-- fixture's `WHERE`-beats-`SET` verdict is not the whole rule: `transformUpdateStmt`
-- runs the FROM clause, the `WHERE`, then `transformReturningClause`, and only then
-- `transformUpdateTargetList` — under a comment saying the target list comes after the
-- SELECT-like processing is done. So the `SET` list is **last**, and of a
-- `SET`/`RETURNING` pair Postgres reports the `RETURNING`. pglet ran `SET` before
-- `RETURNING` and reported the assignment's `42804` where Postgres says the
-- projection's `42883`.
--
-- The two clauses raise different SQLSTATEs on purpose: with the same code on both,
-- the fixture would score green whichever clause won and would lock nothing.
-- @setup
CREATE TABLE rps (i4 INT4, tx TEXT);
INSERT INTO rps VALUES (7, 'a');
-- @test
-- @expect-error: the RETURNING clause resolves before the SET list, so its operator error wins (42883) over the SET's 42804
UPDATE rps SET tx = COALESCE(i4, tx) RETURNING i4 LIKE 'x%';
