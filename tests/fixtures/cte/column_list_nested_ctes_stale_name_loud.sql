-- The reverse polarity of the nested case: the inner CTE's body cannot reach the outer
-- CTE's pre-rename name either. Before the close this was `42703` naming the *wrong*
-- column (`y`, the outer list's name) rather than `a`.
-- @expect-error: the earlier CTE's body name is gone from the later one too
-- @setup
CREATE TABLE clns (a INT);
INSERT INTO clns VALUES (1);
-- @test
WITH c(x) AS (SELECT a FROM clns), e(y) AS (SELECT a FROM c) SELECT y FROM e;
