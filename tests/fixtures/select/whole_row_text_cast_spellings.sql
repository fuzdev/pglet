-- `CAST(t AS text)` is the same row, an alias names the column, and the row's text is a
-- value an expression can read.
-- @setup
CREATE TABLE wrc (a INT, b TEXT);
INSERT INTO wrc VALUES (1, 'q r');
-- @test
SELECT CAST(t AS text), t::text AS x, length(t::text) AS n, (SELECT d::text FROM (SELECT 5 AS v) d) AS derived
FROM wrc t;
