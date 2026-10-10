-- `EXISTS (…)` in a DEFAULT is a subquery: `0A000 cannot use subquery in DEFAULT
-- expression`, the same verdict the scalar `(SELECT …)` form takes, because both
-- reach `transformSubLink`.
-- @expect-error: EXISTS is a subquery in a DEFAULT (0A000)
-- @test
CREATE TABLE dfk (a INT, b BOOL DEFAULT EXISTS (SELECT 1));
