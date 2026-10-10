-- 18.6 transforms one `AND` argument and coerces it before touching the next, so
-- a non-boolean **left** operand outranks a right operand's own error. Its mirror
-- is the fixture beside this one; no pair of whole-subtree passes yields both.
-- @setup
CREATE TABLE bcint (a INT, t TEXT);
-- @expect-error: argument of AND must be type boolean, not the right operand's 42883
-- @test
CREATE TABLE bcint2 (a INT, b BIGINT, CHECK (a AND b LIKE 'x'));
