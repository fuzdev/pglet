-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- @expect-error: 42803 — column "pb.x" must appear in the GROUP BY clause
-- @test
SELECT pa.x FROM pa JOIN pb ON pa.id = pb.id GROUP BY pa.x ORDER BY pb.x;
