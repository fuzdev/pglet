-- Two self-referencing tables dropped together. A table's own FK dies with it and
-- never blocks, so this succeeds for a reason that is NOT the in-set filter — the
-- control that keeps the filter from being credited with the pre-existing rule.
-- @setup
CREATE TABLE dsf1 (id INT PRIMARY KEY, p INT REFERENCES dsf1(id));
CREATE TABLE dsf2 (id INT PRIMARY KEY, p INT REFERENCES dsf2(id));
CREATE TABLE dsfkeep (id INT);
INSERT INTO dsf1 VALUES (1, NULL);
INSERT INTO dsf2 VALUES (1, NULL);

-- @test
DROP TABLE dsf1, dsf2;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'dsf1'
       OR table_name = 'dsf2'
       OR table_name = 'dsfkeep');
