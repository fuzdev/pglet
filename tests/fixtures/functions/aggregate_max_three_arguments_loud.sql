-- `max` has no three-argument signature.
-- @setup
CREATE TABLE agn (a INT, b INT);
INSERT INTO agn VALUES (1, 2);
-- @test
-- @expect-error: function max(integer, integer, integer) does not exist (42883)
SELECT max(a, b, a) FROM agn;
