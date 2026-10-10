-- A CTE, a scalar subquery, a derived table and the outer query read one transaction
-- start and one statement start: the level a read sits at does not move it.
-- @setup
CREATE TABLE ccs (id INT);
INSERT INTO ccs VALUES (1), (2), (3);
-- @test
WITH w AS (SELECT now() AS x, statement_timestamp() AS s)
SELECT ccs.id,
       w.x = now() AS cte_is_outer,
       (SELECT now()) = now() AS subquery_is_outer,
       (SELECT now() FROM ccs c2 WHERE c2.id = ccs.id) = w.x AS correlated_is_cte,
       d.y = w.x AS derived_is_cte,
       w.s = statement_timestamp() AS statement_cte_is_outer,
       (SELECT statement_timestamp()) = d.t AS statement_subquery_is_derived
FROM ccs, w, (SELECT CURRENT_TIMESTAMP AS y, statement_timestamp() AS t) d
ORDER BY ccs.id;
