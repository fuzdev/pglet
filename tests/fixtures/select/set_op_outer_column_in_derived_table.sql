-- A derived table inside an expression subquery can still name the enclosing level, and
-- its set operation's column takes that column's `integer`: the two rows are one value.
-- @setup
CREATE TABLE sdt (i INT);
INSERT INTO sdt VALUES (7);
-- @test
SELECT (SELECT count(*) FROM (SELECT i UNION SELECT '07') s) AS n FROM sdt;
