-- A name written twice is one deletion, not two: 18.6 accepts `DROP TABLE dt, dt`.
-- (The duplicate survives into the NOTICE count for an `IF EXISTS` skip, which is a
-- different question — the skip is per list element and the deletion set is a set.)
-- @setup
CREATE TABLE dtdup (a INT);
CREATE TABLE dtdupkeep (a INT);

-- @test
DROP TABLE dtdup, dtdup;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'dtdup'
       OR table_name = 'dtdupkeep');
