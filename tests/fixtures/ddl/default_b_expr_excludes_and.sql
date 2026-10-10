-- `AND`/`OR` are `a_expr` productions, so a column constraint's `DEFAULT` stops at
-- them. The threshold this replaced already agreed here — the row is kept because the
-- mode has to keep agreeing, and dropping the `And | Or` break silently accepts a
-- statement 18.6 refuses.
-- @expect-error: AND is excluded from a column constraint's DEFAULT
-- @test
CREATE TABLE def_and (a INT, b BOOL DEFAULT 5 = 4 AND true);
