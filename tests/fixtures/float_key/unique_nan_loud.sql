-- `NaN = NaN` in the opclass, so a second NaN is a duplicate.
-- @profile-lock
-- @expect-error: NaN = NaN (23505)
-- @expect-message
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
INSERT INTO fku VALUES (1, 'NaN');
-- @test
INSERT INTO fku VALUES (2, 'NaN');
