-- The other direction widens: the `real` `1.1` is not the `float8` parent `1.1`.
-- @profile-lock
-- @expect-error: the real 1.1 has no float8 parent (23503)
-- @expect-message
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkc48 (x REAL REFERENCES fkp8 (x));
INSERT INTO fkc48 VALUES (1.5), ('NaN'), (0);
-- @test
INSERT INTO fkc48 VALUES (1.1);
