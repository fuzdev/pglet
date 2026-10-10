-- A SET-RHS eval error (integer division by zero) now propagates instead of
-- being swallowed (which silently kept the column's old value and "succeeded").
-- Postgres raises 22012 here; pglet must too, not skip the assignment.
-- @setup
CREATE TABLE e (n INT);
INSERT INTO e VALUES (5);
-- @expect-error: division by zero in a SET RHS fails loud (22012), not swallowed
-- @test
UPDATE e SET n = n / 0;
