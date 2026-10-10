-- The comparison is on an indexed column, so the accelerated planner lifts it out of
-- the `WHERE` into the scan. It still types the parameter ahead of the null test.
-- @setup
CREATE TABLE up_i (id INT PRIMARY KEY, name TEXT, tags TEXT[]);
CREATE INDEX up_i_tags ON up_i USING gin (tags);
INSERT INTO up_i VALUES (1, 'x', ARRAY['a']), (2, 'y', ARRAY['b']);
-- @param $1 int4 1
-- @profile-lock
-- @test
SELECT name FROM up_i WHERE id = $1 AND $1 IS NOT NULL;
