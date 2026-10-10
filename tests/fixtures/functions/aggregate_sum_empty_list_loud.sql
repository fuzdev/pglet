-- `sum` has no zero-argument signature, so `sum()` resolves nothing.
-- @setup
CREATE TABLE agn (a INT, b INT);
-- @test
-- @expect-error: function sum() does not exist (42883)
SELECT sum() FROM agn;
