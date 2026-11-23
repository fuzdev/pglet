-- pglet-stricter, and loud: an advisory call in the body beside a data-modifying CTE is
-- refused `0A000` (18.6 answers per row), the parser admitting only a read-only body there
-- and an advisory projection not being one (`EDGES.md` row 250). The lock is given back in
-- the same projection, so nothing outlives the test on 18.6. Documentary — 18.6 accepts,
-- so any pglet error passes.
-- @setup
CREATE TABLE adm (a INT);
INSERT INTO adm VALUES (1);
-- @test
-- @expect-error: an advisory-lock call, after data-modifying WITH query "d"
WITH d AS (DELETE FROM adm RETURNING a)
SELECT pg_advisory_lock(a), pg_advisory_unlock(a) FROM d;
