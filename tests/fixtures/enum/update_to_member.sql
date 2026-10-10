-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id) VALUES (1);

-- @test
UPDATE cell SET visibility = 'public' WHERE id = 1;
SELECT id, visibility::text AS visibility FROM cell ORDER BY id;
