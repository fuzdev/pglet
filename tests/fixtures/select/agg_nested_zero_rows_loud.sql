-- The nesting is refused over a table with no rows: the verdict is the statement's, not
-- an evaluation's.
-- @setup
CREATE TABLE t (k INT, i INT);
-- @expect-error: aggregate function calls cannot be nested
-- @expect-message
-- @test
SELECT avg(avg(i)) FROM t WHERE false;
