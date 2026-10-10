-- The other half of the interleave, one constraint down: within a *single* constraint
-- the expression is cooked **before** its name is checked, so a duplicate name on an
-- ill-typed CHECK reports the type error. Together with the fixture above this pins the
-- order as per-constraint cook-then-name rather than either whole-list pass.
-- @expect-error: operator does not exist integer ~~ text
-- @test
CREATE TABLE ctce (a INT, CONSTRAINT n CHECK (a > 0), CONSTRAINT n CHECK (a LIKE 'x'));
