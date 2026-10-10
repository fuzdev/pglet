-- The CTE read by an `EXISTS` after the `WHERE`'s cast is still analysed first, so the
-- cast converts a text value, and `false AND …` converts it nowhere.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v) SELECT count(*) AS n FROM cell WHERE false AND vis = $1::cell_visibility AND EXISTS (SELECT 1 FROM a);
