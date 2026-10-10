-- 18.6 analyzes `HAVING` before `GROUP BY`, so the `HAVING`'s text use fixes `$1` first
-- and the grouping key's conversion is on the value — over no row, none.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT);
INSERT INTO cell VALUES (1);

-- @param $1 text secret
-- @test
SELECT count(*) AS n FROM cell WHERE false GROUP BY $1::cell_visibility HAVING $1::text = 'x';
