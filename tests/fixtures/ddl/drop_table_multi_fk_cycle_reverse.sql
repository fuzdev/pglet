-- The reverse written order of `drop_table_multi_fk_cycle.sql`. The pair is the
-- discriminator: no per-name loop passes a mutual FK cycle in EITHER order, so a
-- single-target implementation is red on both while the two-phase one is green on
-- both.
-- @setup
CREATE TABLE mfr1 (id INT PRIMARY KEY, o INT);
CREATE TABLE mfr2 (id INT PRIMARY KEY, o INT);
CREATE TABLE mfrkeep (id INT);
ALTER TABLE mfr1 ADD CONSTRAINT mfr1_o_fkey FOREIGN KEY (o) REFERENCES mfr2(id);
ALTER TABLE mfr2 ADD CONSTRAINT mfr2_o_fkey FOREIGN KEY (o) REFERENCES mfr1(id);

-- @test
DROP TABLE mfr2, mfr1;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'mfr1'
       OR table_name = 'mfr2'
       OR table_name = 'mfrkeep');
