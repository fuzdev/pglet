-- A bare string literal compared against an INT column is coerced to int
-- unconditionally, so an unparseable spelling fails loud (`22P02`) — Postgres
-- resolves the unknown literal to the column type and errors at run time, rather
-- than the old silent 0-row match.
-- @setup
CREATE TABLE t (n INT);
INSERT INTO t VALUES (5);
-- @test
-- @expect-error: unparseable string vs int column errors 22P02, not a silent 0-row match
SELECT * FROM t WHERE n = 'abc';
