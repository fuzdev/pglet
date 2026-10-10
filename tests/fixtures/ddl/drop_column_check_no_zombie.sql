-- Re-adding a dropped column must not resurrect the CHECK that referenced it. With
-- the constraint left in the catalog, pglet accepted the re-add and then raised
-- 23514 on a value the (now non-existent) constraint forbade.
-- @setup
CREATE TABLE dcnz (id INT PRIMARY KEY, a INT, CHECK (a > 0));
ALTER TABLE dcnz DROP COLUMN a;

-- @profile-lock
-- @test
ALTER TABLE dcnz ADD COLUMN a INT;
INSERT INTO dcnz VALUES (1, -1);
SELECT id, a FROM dcnz ORDER BY id;
