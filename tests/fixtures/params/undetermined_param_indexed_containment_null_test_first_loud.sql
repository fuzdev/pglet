-- The neighbour of the indexed-containment optional filter, operands swapped. A GIN
-- containment stays in the recheck filter above its scan, so the source order of the two
-- operands survives index selection and the refusal is the same on both build profiles.
-- @setup
CREATE TABLE up_i (id INT PRIMARY KEY, name TEXT, tags TEXT[]);
CREATE INDEX up_i_tags ON up_i USING gin (tags);
INSERT INTO up_i VALUES (1, 'x', ARRAY['a']), (2, 'y', ARRAY['b']);
-- @param $1 text[] {a}
-- @profile-lock
-- @test
-- @expect-error: the null test is reached before the containment that types the parameter: 42P08
-- @expect-message
SELECT name FROM up_i WHERE ($1 IS NULL OR tags @> $1) ORDER BY id;
