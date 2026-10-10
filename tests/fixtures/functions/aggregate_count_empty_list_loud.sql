-- `count()` resolves the zero-argument aggregate `count(*)` is, and is refused for the
-- spelling.
-- @setup
CREATE TABLE agn (a INT, b INT);
-- @test
-- @expect-error: count(*) must be used to call a parameterless aggregate function (42809)
-- @expect-message
SELECT count() FROM agn;
