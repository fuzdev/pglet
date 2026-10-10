-- Which error wins between a sub-plan in the target list and the `WHERE` after it. 18.6
-- analyzes the target list — the sub-select whole, its unbound `bogus` included — before
-- the `WHERE`, so the answer is `42703`, not the `WHERE`'s `22P02`. The inner table has
-- one column, the shape a sub-plan-level binding check alone refused.
-- @setup
CREATE TABLE cto_p (a INT, b INT);
CREATE TABLE cto_q (c INT);
INSERT INTO cto_p VALUES (1, 10);
INSERT INTO cto_q VALUES (5);
-- @expect-error: the target list's unbound name is analyzed before the WHERE — 42703
-- @test
SELECT (SELECT bogus FROM cto_q) FROM cto_p WHERE a = 'x';
