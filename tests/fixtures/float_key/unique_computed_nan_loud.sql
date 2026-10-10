-- A NaN arithmetic produced — `Infinity - Infinity`, whose sign bit is set on the
-- oracle's platform — duplicates the NaN a literal wrote.
-- @profile-lock
-- @expect-error: every NaN is one key (23505)
-- @expect-message
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
INSERT INTO fku VALUES (1, 'NaN');
-- @test
INSERT INTO fku VALUES (2, 'Infinity'::float8 - 'Infinity'::float8);
