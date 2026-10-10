-- The guarded conversion: the `CASE` condition's `$1 = 'x'` fixes `$1` as `text` ahead of
-- the result's `$1::<enum>`, which is then a conversion only the taken branch reaches.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');

-- @param $1 text secret
-- @test
SELECT (CASE WHEN $1 = 'x' THEN $1::cell_visibility END)::text AS v;
