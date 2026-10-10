-- Its control: a guard resolving FALSE leaves the probe to do the narrowing, so
-- the OR is the containment alone.
-- @setup
CREATE TABLE bcgin2 (id INT, tags TEXT[]);
CREATE INDEX bcgin2_ix ON bcgin2 USING gin (tags);
INSERT INTO bcgin2 VALUES (1, '{a}'), (2, '{b}'), (3, '{c}');
-- @profile-lock
-- @test
SELECT id FROM bcgin2 WHERE tags @> '{a}' OR 'f' ORDER BY id;
