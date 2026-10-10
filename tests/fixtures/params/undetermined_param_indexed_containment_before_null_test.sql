-- The containment spelling, over a GIN index, as an optional filter.
-- @setup
CREATE TABLE up_i (id INT PRIMARY KEY, name TEXT, tags TEXT[]);
CREATE INDEX up_i_tags ON up_i USING gin (tags);
INSERT INTO up_i VALUES (1, 'x', ARRAY['a']), (2, 'y', ARRAY['b']);
-- @param $1 text[] {a}
-- @profile-lock
-- @test
SELECT name FROM up_i WHERE (tags @> $1 OR $1 IS NULL) ORDER BY id;
