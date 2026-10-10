-- Clause order inside a statement, the level above expression source order.
-- `analyze.c`'s `transformUpdateStmt` runs `transformWhereClause` **before**
-- `transformUpdateTargetList`, so when both clauses are ill-typed Postgres reports the
-- `WHERE` one. pglet gated the `SET` list first and reported `42804` where Postgres
-- says `42883`.
-- @setup
CREATE TABLE wps (i4 INT, t TEXT, d DATE);
INSERT INTO wps VALUES (7, NULL, '2026-01-01');
-- @test
-- @expect-error: the WHERE clause resolves first, so its operator error wins (42883)
UPDATE wps SET t = COALESCE(i4, t) WHERE d LIKE 'x%';
