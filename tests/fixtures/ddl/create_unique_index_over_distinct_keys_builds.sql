-- The accept side of the `CREATE UNIQUE INDEX` build gate, and the three shapes whose
-- rows *look* duplicated to a gate that does not read the index's own equality:
--
--   `cuok_ok`    distinct keys — the plain control.
--   `cuok_part`  a **partial** index whose duplicates are all outside its predicate.
--   `cuok_nulls` two NULL keys under the default `NULLS DISTINCT`, which are not
--                uniqueness candidates (the `NULLS NOT DISTINCT` polarity is refused on
--                both engines, and is `EDGES.md` row 92's enforcement half).
--
--   `cuok_taken`  the **`IF NOT EXISTS` skip over duplicate rows** — 18.6 checks the
--                name inside the same `index_create` that builds, and the skip wins, so
--                this reports success on both engines. It is the control for the build
--                gate's *position*: a gate moved ahead of the name conflict makes it a
--                `23505` here and reddens this fixture.
--
-- Each is created (or skipped) on 18.6 and must be here, so a build gate that
-- over-refuses reddens this fixture rather than passing quietly beside the `23505` one.
-- @setup
CREATE TABLE cuok_t (a INT, b INT);
INSERT INTO cuok_t VALUES (1, 7), (2, 8);
CREATE UNIQUE INDEX cuok_ok ON cuok_t (b);
CREATE TABLE cuok_p (a INT, b INT);
INSERT INTO cuok_p VALUES (1, 7), (2, 7), (3, 1);
CREATE UNIQUE INDEX cuok_part ON cuok_p (b) WHERE a = 3;
CREATE TABLE cuok_n (a INT);
INSERT INTO cuok_n VALUES (NULL), (NULL);
CREATE UNIQUE INDEX cuok_nulls ON cuok_n (a);
CREATE TABLE cuok_d (a INT, b INT);
INSERT INTO cuok_d VALUES (1, 7), (2, 7);
CREATE INDEX cuok_taken ON cuok_d (a);
CREATE UNIQUE INDEX IF NOT EXISTS cuok_taken ON cuok_d (b);
-- @test
SELECT indexname::text AS indexname, tablename::text AS tablename
FROM pg_indexes WHERE tablename::text IN ('cuok_t', 'cuok_p', 'cuok_n', 'cuok_d')
ORDER BY indexname::text ASC
