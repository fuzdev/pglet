-- `sum` has no two-argument signature.
-- @setup
CREATE TABLE agn (a INT, b INT);
INSERT INTO agn VALUES (1, 2);
-- @test
-- @expect-error: function sum(integer, integer) does not exist (42883)
SELECT sum(a, b) FROM agn;
