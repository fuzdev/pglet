-- ...and a value no parent holds is `23503`.
-- @profile-lock
-- @expect-error: no parent 2.5 (23503)
-- @expect-message
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkc88 (x FLOAT8 REFERENCES fkp8 (x));
-- @test
INSERT INTO fkc88 VALUES (2.5);
