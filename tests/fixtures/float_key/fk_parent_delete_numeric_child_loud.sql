-- The parent side reads the same equality: the `real` `1.1` is referenced by the
-- `numeric` child `1.1`, so deleting it is `23503`.
-- @profile-lock
-- @expect-error: the numeric 1.1 references the real 1.1 (23503)
-- @expect-message
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkcn4 (x NUMERIC REFERENCES fkp4 (x));
INSERT INTO fkcn4 VALUES (1.1);
-- @test
DELETE FROM fkp4 WHERE x = 1.1::real;
