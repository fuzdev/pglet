-- The empty-means-NULL idiom: the condition fixes `$1` as `text`, so an empty string takes
-- the `NULL` branch and the enum conversion in the other one is never reached.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT, vis cell_visibility);
INSERT INTO cell VALUES (1, 'public');

-- @param $1 text ''
-- @test
UPDATE cell SET vis = CASE WHEN $1 = '' THEN NULL ELSE $1::cell_visibility END WHERE id = 1
RETURNING id, vis::text AS vis;
