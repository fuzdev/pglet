-- pglet-stricter, and loud: the call answers only as a top-level item of a `SELECT` with
-- no `FROM`, the one position a host takes the writer for, so a per-row call is `0A000`
-- here where 18.6 takes the lock per row (`EDGES.md` row 250). Documentary — 18.6
-- accepts, so any pglet error passes; what it holds is that the call never answers a
-- lock nothing took.
-- @setup
CREATE TABLE axl (a INT);
INSERT INTO axl VALUES (1), (2);
-- @test
-- @expect-error: pg_advisory_xact_lock over rows is refused here
SELECT pg_advisory_xact_lock(a) FROM axl;
