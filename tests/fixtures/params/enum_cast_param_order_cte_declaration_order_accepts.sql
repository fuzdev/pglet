-- Two CTEs are analysed in declaration order, not in the order the statement reads
-- them: `a`'s text use is first although `FROM b, a` reaches `b` first. `b` reads an
-- empty table, so its conversion runs over no row.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility, note TEXT);
INSERT INTO cell VALUES (1, 'public', 'a'), (2, 'private', 'b');
CREATE TABLE cell_empty (id INT);
CREATE TABLE cell_log (note TEXT);

-- @param $1 text secret
-- @test
WITH a AS (SELECT $1::text AS v), b AS (SELECT ($1::cell_visibility)::text AS w FROM cell_empty) SELECT * FROM b, a;
