-- `pg_indexes.indexdef`'s **predicate** is the fourth reader of the one Catalog
-- rendering, and the only index position that agrees: 18.6 elides a no-op typmod cast
-- in an index *key* (`coerce_type` returns the argument when type and typmod both
-- match) where pglet keeps it, so the key position stays divergent at the scalar
-- spelling as much as the array one and is deliberately not asserted here.
-- @setup
CREATE TABLE tmix (id INT, a varchar(3)[]);
CREATE INDEX tmix_pred ON tmix (id) WHERE a <> '{zzz}'::varchar(3)[];
-- @no-declared-types: 18.6 sends `name` (19) for `indexname` and pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT indexname, indexdef FROM pg_indexes WHERE tablename = 'tmix' ORDER BY indexname;
