-- @setup
CREATE TABLE cell (id INT, refs TEXT[]);
INSERT INTO cell VALUES (1, ARRAY['x', 'y']), (2, ARRAY['w']), (3, ARRAY['w', 'q']);

-- @test
SELECT id FROM cell WHERE refs <@ ARRAY['w', 'q', 'r'] ORDER BY id;
