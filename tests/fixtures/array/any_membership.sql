-- @setup
CREATE TABLE cell (id INT, name TEXT);
INSERT INTO cell VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd');

-- @test
SELECT id FROM cell WHERE id = ANY(ARRAY[2, 4]) ORDER BY id;
