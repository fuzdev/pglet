-- `ALL` is the no-op quantifier and saves no extra argument.
-- @setup
CREATE TABLE agn (a INT, b INT);
INSERT INTO agn VALUES (1, 2);
-- @test
-- @expect-error: function avg(integer, integer) does not exist (42883)
SELECT avg(ALL a, b) FROM agn;
