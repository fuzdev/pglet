-- 18.6 analyzes an `UPDATE`'s `WHERE` before its `SET` list, so the `WHERE`'s text use
-- fixes `$1` first — whatever order the statement is written in — and the `SET` list's
-- conversion reaches only the rows the guard keeps: here none.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility);
INSERT INTO cell VALUES (1, 'public');

-- @param $1 text secret
-- @test
UPDATE cell SET vis = $1::cell_visibility WHERE $1 <> 'secret' RETURNING id;
