-- @setup
CREATE TABLE cell (id INT, refs TEXT[]);
INSERT INTO cell VALUES (1, ARRAY['a', 'b']), (2, '{c,d}'), (3, NULL), (4, ARRAY[]::text[]);

-- @test
SELECT id, refs FROM cell ORDER BY id;
