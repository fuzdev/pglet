-- Under `WITH RECURSIVE` the list is analysed in dependency order
-- (`TopologicalSort`): `b` reads `a`, so `a` — declared second — is analysed first, and
-- its text use fixes `$1`.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH RECURSIVE b AS (SELECT ($1::cell_visibility)::text AS w FROM cell_empty WHERE EXISTS (SELECT 1 FROM a)), a AS (SELECT $1::text AS v) SELECT * FROM b;
