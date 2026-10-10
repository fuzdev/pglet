-- Control: a qualifier no level names is `42P01`, over an inner level of two empty
-- tables, so no row ever reaches the eval-time resolver.
-- @setup
CREATE TABLE ctq_p (a INT, b INT);
CREATE TABLE ctq_e (c INT);
INSERT INTO ctq_p VALUES (1, 10);
-- @expect-error: a qualifier no level names — 42P01
-- @test
SELECT (SELECT zz.b FROM ctq_e x, ctq_e y) FROM ctq_p;
