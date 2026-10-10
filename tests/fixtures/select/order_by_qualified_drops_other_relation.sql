-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- The join is deliberately non-equi in its `x` values (pa.x ascends, pb.x
-- descends) so sorting by pa.x and by pb.x give opposite orders — a fixture whose
-- two columns are equal by the join predicate cannot observe this bug at all.
-- The output column is left named `x`, colliding with pb's bare column name:
-- a sort placement that compares ORDER BY references to output names by bare name
-- sees `x` as already projected and sorts by pa.x instead.
-- @test
SELECT pa.x FROM pa JOIN pb ON pa.id = pb.id ORDER BY pb.x;
