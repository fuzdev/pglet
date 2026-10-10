-- A `WITH` query is analysed before every clause of the statement that reads it, so the
-- CTE's text use fixes `$1` first even where the reference sits in a scalar subquery of
-- the select list, after the cast: the select list's conversion then runs on the value,
-- and under `WHERE false` it runs nowhere. (An ordinary read-only CTE inlines at its
-- reference; ranking it there put the cast first and refused the label at `Bind`.)
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v) SELECT ($1::cell_visibility)::text AS w, (SELECT v FROM a) AS v FROM cell WHERE false;
