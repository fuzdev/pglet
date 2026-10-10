-- Each CTE is analysed in declaration order, so a later one sees the earlier one under
-- its **renamed** names.
-- @setup
CREATE TABLE cln (a INT);
INSERT INTO cln VALUES (1), (2);
-- @test
WITH c(x) AS (SELECT a FROM cln), e(y) AS (SELECT x FROM c) SELECT y FROM e ORDER BY y;
