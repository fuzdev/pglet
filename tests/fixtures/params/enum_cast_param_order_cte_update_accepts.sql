-- An `UPDATE`'s `WITH` list is analysed before its `WHERE`, so the CTE's text use is
-- first and the `WHERE`'s conversion is decided on the value — here, reached by no row.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v) UPDATE cell SET note = 'x' WHERE false AND vis = $1::cell_visibility AND EXISTS (SELECT 1 FROM a) RETURNING id;
