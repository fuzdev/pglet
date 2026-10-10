-- `check_recursive_shape` folds the recursive term against the seed's descriptors, so
-- what the seed's descriptor carries decides the fold. With the identity dropped the
-- fold was `text` against `text` and the query was accepted; it is the pair 18.6 has no
-- common type for.
--
-- The **mirror** of `enum/recursive_cte_text_term_loud`, which puts the text on the seed
-- side and reaches the same refusal through the degraded (no-self-reference) path. This
-- one is genuinely recursive, so it is the rebuild's own reader.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'ok'), (2, 'sad'), (3, 'happy');

-- @test
-- @expect-error: no common type for mood and text
WITH RECURSIVE r(z) AS (
  SELECT m FROM t
  UNION ALL
  SELECT z || '' FROM r WHERE false
)
SELECT 1 AS one FROM r;
