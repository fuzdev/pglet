-- A self-referencing FK dies with its own table, so it never blocks the drop —
-- the "who owns it" half of the dependency rule.
-- @setup
CREATE TABLE dtsr (id INT PRIMARY KEY, parent_id INT REFERENCES dtsr(id));
INSERT INTO dtsr VALUES (1, NULL), (2, 1);

-- @test
DROP TABLE dtsr;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public' AND table_name = 'dtsr';
