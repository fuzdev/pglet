-- An `ON CONFLICT` action's `WHERE` holding the only occurrence.
-- @setup
CREATE TABLE up_c (id INT PRIMARY KEY, name TEXT);
INSERT INTO up_c VALUES (1, 'x');
-- @param $1 text NULL
-- @profile-lock
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
INSERT INTO up_c (id, name) VALUES (1, 'q') ON CONFLICT (id) DO UPDATE SET name = 'z' WHERE $1 IS NULL;
