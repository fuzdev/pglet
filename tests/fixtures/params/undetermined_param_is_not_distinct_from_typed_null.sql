-- The neighbour of the `IS DISTINCT FROM NULL` refusal: against a *typed* NULL the
-- comparison is an ordinary operator, which types the parameter as `text`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT name FROM up_t WHERE $1 IS NOT DISTINCT FROM NULL::text ORDER BY id;
