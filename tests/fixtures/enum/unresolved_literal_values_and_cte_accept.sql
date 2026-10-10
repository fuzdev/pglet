-- The same unknown-literal rule at the two doors beside the set operation's: a FROM-clause
-- `VALUES` column and a CTE's fold.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
WITH c AS (SELECT 'sad' AS x UNION ALL SELECT m FROM t)
SELECT (SELECT count(*) FROM c) AS cte_rows,
       (SELECT count(*) FROM (VALUES ('sad'), ((SELECT m FROM t LIMIT 1))) w(x)) AS values_rows;
