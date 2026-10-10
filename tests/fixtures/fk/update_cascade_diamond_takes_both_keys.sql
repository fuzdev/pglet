-- A child reached by two changing parents in one statement — `da` directly, and `dg`,
-- whose key cascades from `da` — takes both repoints.
-- @setup
CREATE TABLE da (k int PRIMARY KEY);
CREATE TABLE dg (k int PRIMARY KEY REFERENCES da ON UPDATE CASCADE);
CREATE TABLE dc (id int, a int REFERENCES da ON UPDATE CASCADE, b int REFERENCES dg ON UPDATE CASCADE);
INSERT INTO da VALUES (1);
INSERT INTO dg VALUES (1);
INSERT INTO dc VALUES (1, 1, 1);
UPDATE da SET k = 5;
-- @test
-- @profile-lock
SELECT id, a, b FROM dc;
