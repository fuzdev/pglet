-- A three-table cycle (r1 → r2 → r3 → r1) dropped in a written order that is neither
-- the cycle's nor its reverse. The filter is over the whole target SET, not a
-- neighbour-by-neighbour walk, so the permutation is irrelevant.
-- @setup
CREATE TABLE ttc1 (id INT PRIMARY KEY, o INT);
CREATE TABLE ttc2 (id INT PRIMARY KEY, o INT);
CREATE TABLE ttc3 (id INT PRIMARY KEY, o INT);
CREATE TABLE ttckeep (id INT);
ALTER TABLE ttc1 ADD CONSTRAINT ttc1_o_fkey FOREIGN KEY (o) REFERENCES ttc2(id);
ALTER TABLE ttc2 ADD CONSTRAINT ttc2_o_fkey FOREIGN KEY (o) REFERENCES ttc3(id);
ALTER TABLE ttc3 ADD CONSTRAINT ttc3_o_fkey FOREIGN KEY (o) REFERENCES ttc1(id);

-- @test
DROP TABLE ttc2, ttc1, ttc3;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'ttc1'
       OR table_name = 'ttc2'
       OR table_name = 'ttc3'
       OR table_name = 'ttckeep');
