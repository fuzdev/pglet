-- pglet-stricter, and loud: an advisory call under a `LIMIT` is refused `0A000` (18.6
-- answers `false` per row it reaches), since the calls' count there is not the rows'
-- count the projection evaluates (`EDGES.md` row 250). The unlock takes nothing on 18.6,
-- so nothing outlives the test there. Documentary — 18.6 accepts, so any pglet error
-- passes.
-- @setup
CREATE TABLE aul (a INT);
INSERT INTO aul VALUES (1), (2);
-- @test
-- @expect-error: an advisory call under a LIMIT is refused here
SELECT pg_advisory_unlock(a) FROM aul LIMIT 1;
