-- The float is the leading part of the unique key and the second column of the table.
-- @profile-lock
-- @expect-error: (NaN, 1) is taken (23505)
-- @expect-message
-- @setup
CREATE TABLE fkm (t TEXT, f FLOAT8, a INT, r REAL);
CREATE UNIQUE INDEX fkm_fa ON fkm (f, a);
CREATE INDEX fkm_ar ON fkm (a, r);
CREATE INDEX fkm_tfr ON fkm (r, t, f);
INSERT INTO fkm VALUES ('p', 1.5, 1, 1.5), ('q', 1.5, 2, 'NaN'), ('r', 'NaN', 1, 0);
-- @test
INSERT INTO fkm VALUES ('s', 'NaN', 1, 9);
