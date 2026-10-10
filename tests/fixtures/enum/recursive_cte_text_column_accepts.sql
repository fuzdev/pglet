-- The over-refusal polarity of the rebuild: carrying the seed's identity must not invent
-- one. A genuinely recursive CTE over a plain `TEXT` column, in a database that holds an
-- enum, still compares by bytes and still answers.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'ok', 'b'), (2, 'sad', 'a'), (3, 'happy', 'c');

-- @test
WITH RECURSIVE r(z) AS (
  SELECT s FROM t
  UNION ALL
  SELECT z FROM r WHERE false
)
SELECT count(*) AS n FROM r WHERE z > 'a';
