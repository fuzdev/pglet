-- An `UPDATE` onto a key another row holds is `23505`.
-- @profile-lock
-- @expect-error: the updated key collides (23505)
-- @expect-message
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
INSERT INTO fku VALUES (1, 1.5), (2, 2.5);
-- @test
UPDATE fku SET x = 1.5 WHERE n = 2;
