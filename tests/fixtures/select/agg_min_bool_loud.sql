-- `min`/`max` are hard-coded catalog facts, NOT "any comparable type": `bool` has
-- complete comparison operators, yet Postgres registered no `min(boolean)`. pglet
-- used to fold it through the permissive comparator and return a value.
-- @setup
CREATE TABLE mb (flag BOOL);
INSERT INTO mb VALUES (true), (false);
-- @test
-- @expect-error: no min(boolean)
SELECT MIN(flag) FROM mb;
