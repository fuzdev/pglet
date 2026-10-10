-- Casting to a user ENUM type, both directly (`'public'::cell_visibility`) and
-- via the text round-trip the spine uses (`'public'::text::cell_visibility`),
-- then back to text in the projection. The CAST path resolves the enum name
-- against the catalog (enums are stored as their TEXT label); a member value
-- reads back unchanged and compares equal to the same value in the column.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id, visibility) VALUES (1, 'private'), (2, 'public'), (3, 'public');

-- @test
SELECT id, ('public'::text::cell_visibility)::text AS lit_cast
FROM cell
WHERE visibility = 'public'::cell_visibility
ORDER BY id;
