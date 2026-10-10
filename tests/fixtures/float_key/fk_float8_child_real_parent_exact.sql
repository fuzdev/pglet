-- The `float8` children a `real` parent does hold: values the narrow width represents
-- exactly, a NaN, `-0`, and the widened `real` `1.1` itself.
-- @profile-lock
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkc84 (x FLOAT8 REFERENCES fkp4 (x));
INSERT INTO fkc84 VALUES (1.5), ('NaN'), ('-0'), (1.1::real);
-- @test
SELECT x::text AS shown FROM fkc84 ORDER BY x;
