-- ...and the `bigint` child `16777217` references the `real` `16777216`.
-- @profile-lock
-- @expect-error: 16777217 references the real 16777216 (23503)
-- @expect-message
-- @setup
CREATE TABLE fkp8 (note TEXT, x FLOAT8 PRIMARY KEY);
CREATE TABLE fkp4 (note TEXT, x REAL PRIMARY KEY);
INSERT INTO fkp8 (x) VALUES (0), ('NaN'), (1.5), (0.3), (9007199254740992), (1.1);
INSERT INTO fkp4 (x) VALUES (0), ('NaN'), (1.5), (1.1), (16777216);
CREATE TABLE fkci4 (x BIGINT REFERENCES fkp4 (x));
INSERT INTO fkci4 VALUES (16777217);
-- @test
DELETE FROM fkp4 WHERE x = 16777216;
