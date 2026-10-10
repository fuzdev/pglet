-- Its refusal half, through the same walk.
-- @setup
CREATE TABLE bcag2 (a INT, b TEXT);
INSERT INTO bcag2 VALUES (1, 'x'), (2, 'y');
-- @expect-error: argument of CASE/WHEN must be type boolean
-- @test
SELECT b, count(CASE WHEN a THEN 1 END) FROM bcag2 GROUP BY b ORDER BY b;
