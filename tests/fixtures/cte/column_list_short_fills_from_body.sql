-- A list shorter than the body's output fills from the body: `analyzeCTE` takes
-- `ctecolnames[i]` while `i < numaliases` and `te->resname` beyond. Not an error.
-- @setup
CREATE TABLE cls (a INT, b TEXT);
INSERT INTO cls VALUES (1, 'x');
-- @test
WITH c(p) AS (SELECT a, b FROM cls) SELECT p, b FROM c;
