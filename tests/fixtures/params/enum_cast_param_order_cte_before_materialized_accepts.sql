-- A data-modifying CTE is materialized and a read-only one inlined, and the two are
-- still analysed in one declaration order: the read-only `a`'s text use is first.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v), d AS (INSERT INTO cell_log SELECT ($1::cell_visibility)::text FROM cell_empty RETURNING note) SELECT * FROM a, d;
