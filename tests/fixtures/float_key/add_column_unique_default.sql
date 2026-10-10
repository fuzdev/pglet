-- `ADD COLUMN … FLOAT8 UNIQUE DEFAULT` over one row back-fills and builds the index;
-- later rows take the default only once.
-- @profile-lock
-- @setup
CREATE TABLE fka (a INT);
INSERT INTO fka VALUES (1);
ALTER TABLE fka ADD COLUMN f FLOAT8 UNIQUE DEFAULT 1.5;
INSERT INTO fka VALUES (3, 2.5);
-- @test
SELECT a, f FROM fka ORDER BY a;
