-- `count` has no two-argument signature: the extra argument is refused, not dropped —
-- over an empty table too, since the call is resolved before any row is read.
-- @setup
CREATE TABLE agn (a INT, b INT);
-- @test
-- @expect-error: function count(integer, integer) does not exist (42883)
SELECT count(a, b) FROM agn;
