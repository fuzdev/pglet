-- `qualifier.*` expands to explicit *qualified* column references at plan time, so
-- it took the same wrong-declared-type path as a written-out `t.c` — every column
-- of `SELECT t.* FROM t` came back declared `text`, including the bool. The bare
-- `SELECT *` form never did (it short-circuits to the input descriptors), which is
-- why the two spellings need separate locks.
-- @setup
CREATE TABLE qst (k INT PRIMARY KEY, x INT, f FLOAT8, b BOOL, t TEXT);
INSERT INTO qst VALUES (1, 10, 1.5, true, 'hi');

-- @test
SELECT qst.* FROM qst;
