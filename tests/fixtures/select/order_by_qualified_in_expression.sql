-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- The qualified reference is nested inside an expression key; the collision with
-- the output name `x` is the same.
-- @test
SELECT pa.x FROM pa JOIN pb ON pa.id = pb.id ORDER BY pb.x + 1;
