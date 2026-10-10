-- Two rows of one statement whose composite keys differ only in zero's sign.
-- @profile-lock
-- @expect-error: (-0, 3) = (0, 3) (23505)
-- @expect-message
-- @setup
CREATE TABLE fkm (t TEXT, f FLOAT8, a INT, r REAL);
CREATE UNIQUE INDEX fkm_fa ON fkm (f, a);
CREATE INDEX fkm_ar ON fkm (a, r);
CREATE INDEX fkm_tfr ON fkm (r, t, f);
INSERT INTO fkm VALUES ('p', 1.5, 1, 1.5), ('q', 1.5, 2, 'NaN'), ('r', 'NaN', 1, 0);
-- @test
INSERT INTO fkm VALUES ('s', '-0', 3, 9), ('t', 0, 3, 9);
