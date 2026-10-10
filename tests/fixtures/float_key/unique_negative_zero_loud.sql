-- `-0` duplicates `0` under a unique `float8` key: `23505`, as `float8eq` decides.
-- @profile-lock
-- @expect-error: -0 = 0 (23505)
-- @expect-message
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
INSERT INTO fku VALUES (1, 0);
-- @test
INSERT INTO fku VALUES (2, '-0');
