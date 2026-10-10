-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- The grouping rule accepts a qualified ORDER BY key that names a grouping column.
-- The loud 42803 on an *ungrouped* qualified key must not over-reject this one.
-- @test
SELECT pa.x FROM pa JOIN pb ON pa.id = pb.id GROUP BY pa.x ORDER BY pa.x DESC;
