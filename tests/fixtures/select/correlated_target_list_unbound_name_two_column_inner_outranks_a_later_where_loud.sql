-- The two-column spelling of the target-list-before-`WHERE` precedence: the sub-plan's
-- unbound `bogus` is 18.6's `42703`, ahead of the `WHERE`'s `22P02`, whatever the inner
-- table's width.
-- @setup
CREATE TABLE ctw_p (a INT, b INT);
CREATE TABLE ctw_q (c INT, d INT);
INSERT INTO ctw_p VALUES (1, 10);
INSERT INTO ctw_q VALUES (5, 6);
-- @expect-error: the target list's unbound name is analyzed before the WHERE — 42703
-- @test
SELECT (SELECT bogus FROM ctw_q) FROM ctw_p WHERE a = 'x';
