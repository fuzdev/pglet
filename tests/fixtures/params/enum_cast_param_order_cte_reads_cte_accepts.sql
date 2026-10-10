-- One CTE read inside another's body was analysed with the earlier `WITH` query, not
-- inside the later one, so `a`'s text use is first.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v), b AS (SELECT v, ($1::cell_visibility)::text AS w FROM a, cell_empty) SELECT * FROM b;
