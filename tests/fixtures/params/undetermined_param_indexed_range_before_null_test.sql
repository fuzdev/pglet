-- The range spelling of the lifted comparison.
-- @setup
CREATE TABLE up_i (id INT PRIMARY KEY, name TEXT, tags TEXT[]);
CREATE INDEX up_i_tags ON up_i USING gin (tags);
INSERT INTO up_i VALUES (1, 'x', ARRAY['a']), (2, 'y', ARRAY['b']);
-- @param $1 int4 1
-- @profile-lock
-- @test
SELECT name FROM up_i WHERE id > $1 AND $1 IS NOT NULL ORDER BY id;
