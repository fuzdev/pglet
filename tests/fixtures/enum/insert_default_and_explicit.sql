-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id) VALUES (1);
INSERT INTO cell (id, visibility) VALUES (2, 'public');

-- @test
SELECT id, visibility::text AS visibility FROM cell ORDER BY id;
