-- The row source is analysed before the `ON CONFLICT` clause.
-- @setup
CREATE TABLE up_c (id INT PRIMARY KEY, name TEXT);
INSERT INTO up_c VALUES (1, 'x');
-- @param $1 text NULL
-- @test
INSERT INTO up_c (id, name) VALUES (1, $1) ON CONFLICT (id) DO UPDATE SET name = 'z' WHERE $1 IS NULL RETURNING name;
