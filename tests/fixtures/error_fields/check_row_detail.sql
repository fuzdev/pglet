-- The write-path 23514 (`ExecConstraints`) carries `s`/`t`/`n` and the same
-- `Failing row contains (…)` DETAIL a not-null violation does.
-- @setup
CREATE TABLE t_c (a INT CHECK (a > 0), z TEXT);

-- @test
-- @expect-error: the row fails the CHECK
-- @expect-message
INSERT INTO t_c VALUES (-1, 'z');
