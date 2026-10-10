-- A wildcard over a **derived table** is that table's own query's column, so the question
-- recurses into it: the branch is `numeric` here as it is in 18.6, through a nested
-- subquery and a level-naming entry both. Left uncertain, the set unified nothing and the
-- projection declared `text` over a `numeric` payload — which the binary leg below is
-- what sees.
-- @setup
CREATE TABLE ssun (n NUMERIC);
INSERT INTO ssun VALUES (1.25);
INSERT INTO ssun VALUES (3.75);
-- @test
SELECT coalesce((SELECT (SELECT * FROM (SELECT o.n AS z) w) + 0), 2) AS a,
       coalesce((SELECT * FROM (SELECT o.n AS z) w), 2) AS b
FROM ssun o ORDER BY 1;
