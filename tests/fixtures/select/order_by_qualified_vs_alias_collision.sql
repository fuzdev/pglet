-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- `AS x` makes the output name collide with pb's bare column name. A qualified
-- ORDER BY reference is two fields, so Postgres skips the output-name branch
-- entirely and reads pb's column regardless of the collision.
-- @test
SELECT pa.x AS x FROM pa JOIN pb ON pa.id = pb.id ORDER BY pb.x;
