-- Composite keys mixing float and non-float parts in both positions, none in the
-- table's column order. NULL float parts are distinct; an `ON CONFLICT` target written
-- in the other order still names the `(f, a)` index.
-- @profile-lock
-- @setup
CREATE TABLE fkm (t TEXT, f FLOAT8, a INT, r REAL);
CREATE UNIQUE INDEX fkm_fa ON fkm (f, a);
CREATE INDEX fkm_ar ON fkm (a, r);
CREATE INDEX fkm_tfr ON fkm (r, t, f);
INSERT INTO fkm VALUES ('p', 1.5, 1, 1.5), ('q', 1.5, 2, 'NaN'), ('r', 'NaN', 1, 0);
INSERT INTO fkm VALUES ('s', NULL, 3, 9), ('t', NULL, 3, 9);
INSERT INTO fkm VALUES ('z', 4, 4, 4) ON CONFLICT (a, f) DO UPDATE SET t = 'never';
INSERT INTO fkm VALUES ('z', 1.5, 1, 4) ON CONFLICT (a, f) DO UPDATE SET t = fkm.t || excluded.t;
UPDATE fkm SET r = '-0' WHERE t = 'q';
-- @test
SELECT t, f::text AS shown, a, r::text AS r_shown FROM fkm ORDER BY t, a;
