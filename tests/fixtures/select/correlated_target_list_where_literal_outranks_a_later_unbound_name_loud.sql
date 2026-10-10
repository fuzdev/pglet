-- The other half of the precedence pair: inside one `WHERE`, `a = 'x'` is analyzed
-- before the `EXISTS` written after it, so 18.6 answers the literal's `22P02` and never
-- reaches the sub-select's unbound `bogus`. A binding check run as the sub-select is
-- lowered answered `42703`.
-- @setup
CREATE TABLE ctv_p (a INT, b INT);
CREATE TABLE ctv_q (c INT);
INSERT INTO ctv_p VALUES (1, 10);
INSERT INTO ctv_q VALUES (5);
-- @expect-error: the WHERE's earlier literal is analyzed before the sub-select — 22P02
-- @test
SELECT a FROM ctv_p WHERE a = 'x' AND EXISTS (SELECT bogus FROM ctv_q);
