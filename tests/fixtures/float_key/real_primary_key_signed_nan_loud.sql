-- A `real` primary key: `-NaN` duplicates `NaN`.
-- @profile-lock
-- @expect-error: -NaN = NaN (23505)
-- @expect-message
-- @setup
CREATE TABLE fkp (b TEXT, x REAL PRIMARY KEY, a INT);
INSERT INTO fkp VALUES ('a', 'NaN', 1);
-- @test
INSERT INTO fkp VALUES ('b', '-NaN', 2);
