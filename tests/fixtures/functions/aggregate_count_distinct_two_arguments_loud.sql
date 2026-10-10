-- The argument count decides ahead of the quantifier: `count(DISTINCT a, b)` is the
-- same refusal as `count(a, b)`.
-- @setup
CREATE TABLE agn (a INT, b INT);
INSERT INTO agn VALUES (1, 2);
-- @test
-- @expect-error: function count(integer, integer) does not exist (42883)
SELECT count(DISTINCT a, b) FROM agn;
