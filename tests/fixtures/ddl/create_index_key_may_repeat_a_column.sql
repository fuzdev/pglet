-- The control that bounds the repeat rule to the **constraint**: `CREATE INDEX` builds
-- its key list through `DefineIndex`, which carries no such test, so a two-segment index
-- over one column is created by both engines and `pg_indexes.indexdef` reads both
-- segments back; the `@setup`'s rows then write without refusal.
-- A gate that deduplicated every name list rather than a constraint's key list would
-- redden here.
-- @setup
CREATE TABLE cikr (a INT, b INT);
CREATE INDEX cikr_ix ON cikr (a, a);
INSERT INTO cikr VALUES (1, 10), (2, 20);
-- @test
SELECT indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'cikr';
