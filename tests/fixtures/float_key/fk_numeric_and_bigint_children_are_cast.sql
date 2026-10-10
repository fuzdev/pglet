-- A `numeric` or integer child of a float parent has no cross-type `=` in the float
-- opfamily, so 18.6 casts it to the parent's type — and to `real` that cast rounds:
-- `1.1` references the `real` `1.1` and `16777217` the `real` `16777216`.
-- @profile-lock
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkcn4 (x NUMERIC REFERENCES fkp4 (x));
CREATE TABLE fkci4 (x BIGINT REFERENCES fkp4 (x));
CREATE TABLE fkcn8 (x NUMERIC REFERENCES fkp8 (x));
CREATE TABLE fkci8 (x BIGINT REFERENCES fkp8 (x));
INSERT INTO fkcn4 VALUES (1.5), (1.1);
INSERT INTO fkci4 VALUES (16777217);
INSERT INTO fkcn8 VALUES (0.30000000000000000001);
INSERT INTO fkci8 VALUES (9007199254740993);
-- @test
SELECT (SELECT count(*) FROM fkcn4) AS n4, (SELECT count(*) FROM fkci4) AS i4,
       (SELECT count(*) FROM fkcn8) AS n8, (SELECT count(*) FROM fkci8) AS i8;
