-- 23502 carries `s`/`t`/`c` and a `Failing row contains (…)` DETAIL built from the
-- whole tuple — so the check has to run after the row is assembled, not per column.
-- @setup
CREATE TABLE t_nn (id INT NOT NULL, name TEXT);

-- @test
-- @expect-error: the omitted NOT NULL column
-- @expect-message
INSERT INTO t_nn (name) VALUES ('a');
