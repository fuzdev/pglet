-- A set operation's branches are analyzed left to right, so the left branch's text use
-- fixes `$1` and the right branch's conversion is on the value — over no row, none.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT);
INSERT INTO cell VALUES (1);

-- @param $1 text secret
-- @test
SELECT $1::text AS v FROM cell WHERE false
UNION ALL SELECT ($1::cell_visibility)::text FROM cell WHERE false;
