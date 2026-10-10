-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');

-- @test
-- @expect-error: invalid input value for enum
INSERT INTO cell (id, visibility) VALUES (1, 'bogus');
