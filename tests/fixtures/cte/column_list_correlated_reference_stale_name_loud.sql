-- The correlated walk's reverse polarity: a correlated qualifier naming the CTE's
-- **pre-rename** column must not resolve.
-- @expect-error: the pre-rename name is not in the correlated chain
-- @setup
CREATE TABLE clcr (a INT);
CREATE TABLE clcrs (k INT, m TEXT);
INSERT INTO clcr VALUES (1), (2);
INSERT INTO clcrs VALUES (1, 'one'), (2, 'two');
-- @test
WITH c(p) AS (SELECT a FROM clcr)
SELECT p, (SELECT m FROM clcrs WHERE clcrs.k = c.a) AS corr FROM c ORDER BY p;
