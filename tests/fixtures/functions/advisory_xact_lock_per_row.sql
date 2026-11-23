-- A call projected over a FROM answers once per row: the statement's own projection,
-- over a FROM no later node follows, is a position the host takes the writer for, and
-- every key is that one writer (`EDGES.md` row 250).
-- @setup
CREATE TABLE axl (a INT);
INSERT INTO axl VALUES (2), (1);
-- @test
SELECT a, pg_advisory_xact_lock(a) AS k FROM axl WHERE a > 0;
