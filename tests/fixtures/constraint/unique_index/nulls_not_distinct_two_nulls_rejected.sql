-- A `NULLS NOT DISTINCT` unique index treats two NULL keys as equal, so the
-- second NULL-keyed insert violates the constraint (contrast the default
-- NULLS DISTINCT `unique_multiple_nulls`, which allows multiple NULLs).
-- @profile-lock
-- @setup
CREATE TABLE nnd_two_nulls (id INT, tag TEXT);
CREATE UNIQUE INDEX ux_nnd_two_nulls ON nnd_two_nulls (tag) NULLS NOT DISTINCT;
INSERT INTO nnd_two_nulls VALUES (1, NULL);

-- @test
-- @expect-error: a second NULL collides under NULLS NOT DISTINCT
INSERT INTO nnd_two_nulls VALUES (2, NULL);
