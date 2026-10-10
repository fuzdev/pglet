-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id) VALUES (1);

-- @test
-- @expect-error: invalid input value for enum
UPDATE cell SET visibility = 'bogus' WHERE id = 1;
