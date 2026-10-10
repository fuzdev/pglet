-- The simple `CASE <operand> WHEN <value>` form is Postgres's `operand = value`,
-- so a mistyped WHEN is `42883`. The simple form called `values_equal` directly,
-- bypassing the operator gate, and silently fell through to the ELSE branch.
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('5');
-- @test
-- @expect-error: no text = integer operator
SELECT CASE name WHEN 5 THEN 'five' ELSE 'other' END FROM t;
