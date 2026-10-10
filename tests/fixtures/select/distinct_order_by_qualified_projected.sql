-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- Matching is by target-entry identity, not spelling: a qualified reference to a
-- projected column satisfies the DISTINCT rule and must still sort correctly.
-- @test
SELECT DISTINCT pa.x AS pax FROM pa JOIN pb ON pa.id = pb.id ORDER BY pa.x DESC;
