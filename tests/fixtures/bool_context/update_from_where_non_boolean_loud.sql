-- `UPDATE … FROM … WHERE` is a third eval-time truth site, reached by neither of
-- the other two.
-- @setup
CREATE TABLE bcuf1 (a INT);
CREATE TABLE bcuf2 (v INT);
INSERT INTO bcuf1 VALUES (1);
INSERT INTO bcuf2 VALUES (7);
-- @expect-error: argument of WHERE must be type boolean
-- @test
UPDATE bcuf1 SET a = 2 FROM bcuf2 WHERE bcuf2.v;
