-- `HAVING 't'` keeps every group; it dropped all of them here.
-- @setup
CREATE TABLE bch (a INT, b TEXT);
INSERT INTO bch VALUES (1, 'x'), (2, 'y');
-- @test
SELECT b, count(*) FROM bch GROUP BY b HAVING 't' ORDER BY b;
