-- 18.6 analyzes an `UPDATE`'s `RETURNING` list before its `SET` list, so a `RETURNING`
-- text use fixes `$1` first and the `SET` conversion is on the value — no row here.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility);
INSERT INTO cell VALUES (1, 'public');

-- @param $1 text secret
-- @test
UPDATE cell SET vis = $1::cell_visibility WHERE false RETURNING $1::text AS p;
