-- The `VALUES` row is analyzed before the `ON CONFLICT` action, so the text column fixes
-- `$1` as `text`; with no conflict the action's conversion is never reached and the row
-- inserts.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT PRIMARY KEY, s TEXT, vis cell_visibility);
INSERT INTO cell VALUES (1, 'a', 'public');

-- @param $1 text secret
-- @test
INSERT INTO cell (id, s) VALUES (5, $1) ON CONFLICT (id) DO UPDATE SET vis = $1::cell_visibility
RETURNING id, s;
