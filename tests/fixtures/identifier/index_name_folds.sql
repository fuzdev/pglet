-- An index name folds, and the fold is visible in `pg_indexes` — the catalog-reading
-- form of the guard, so both sides of the comparison do not move together. Under a
-- non-folding engine the index is named `Ix1` and the row is absent.
-- @setup
CREATE TABLE idxfold (a INT);
CREATE INDEX Ix1 ON idxfold (a);

-- @test
SELECT indexname::text AS indexname
FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'idxfold'
ORDER BY indexname;
