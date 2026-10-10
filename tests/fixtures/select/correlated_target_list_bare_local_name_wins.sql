-- A **bare** name the sub-plan's own level carries binds there, however an enclosing
-- level types the same name: `b` is the inner `text` column, not the outer integer.
-- @setup
CREATE TABLE ctb_p (a INT, b INT);
CREATE TABLE ctb_q (c INT, b TEXT);
INSERT INTO ctb_p VALUES (1, 10), (2, 20);
INSERT INTO ctb_q VALUES (5, 'zz');
-- @test
SELECT a, (SELECT b FROM ctb_q) AS v FROM ctb_p ORDER BY a;
