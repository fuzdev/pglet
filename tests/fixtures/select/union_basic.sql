-- UNION (without ALL) concatenates both sides and removes whole-row duplicates.
-- The row (2, 'b') is produced by both selects and collapses to one.
-- @setup
CREATE TABLE union_basic (id INT, name TEXT);
INSERT INTO union_basic VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT id, name FROM union_basic WHERE id <= 2
UNION
SELECT id, name FROM union_basic WHERE id >= 2
ORDER BY id;
