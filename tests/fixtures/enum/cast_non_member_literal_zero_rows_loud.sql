-- The literal's conversion is in parse analysis, so no row has to reach it: over an empty
-- table, under `WHERE false`, the cast is refused all the same.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT 'q'::mood::text AS v FROM t WHERE false;
