-- @setup
CREATE TABLE cell (id INT, refs TEXT[]);
INSERT INTO cell VALUES (1, ARRAY['x', 'y']), (2, ARRAY['y', 'z']), (3, ARRAY['w']);

-- @test
SELECT id FROM cell WHERE refs @> ARRAY['y'] ORDER BY id;
