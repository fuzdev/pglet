-- `text = boolean` has no operator (`42883`); the permissive fallback returned
-- `false` for every row.
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('true');
-- @test
-- @expect-error: no text = boolean operator
SELECT * FROM t WHERE name = true;
