-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id, visibility) VALUES (1, 'private'), (2, 'public'), (3, 'public');

-- @test
SELECT id FROM cell WHERE visibility = 'public' ORDER BY id;
