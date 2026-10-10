-- The in-set half of the phase-2 filter. `isb.x` defaults from a sequence `isa` owns,
-- so `DROP TABLE isa` alone is `2BP01` (see `drop_table_multi_outside_default_loud`) —
-- but the column's own table is in the list, so the dependent goes with it and the
-- statement succeeds. The relation that must be in the set is the one the dependent is
-- declared ON, never the one it references.
-- @setup
CREATE TABLE isa (id SERIAL, v INT);
CREATE TABLE isb (x INT DEFAULT nextval('isa_id_seq'));
CREATE TABLE iskeep (z INT);

-- @test
DROP TABLE isa, isb;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'isa'
       OR table_name = 'isb'
       OR table_name = 'iskeep');
