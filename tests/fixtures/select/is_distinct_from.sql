-- Null-safe comparison covering all four NULL/non-NULL combinations of two
-- columns. Every projected `IS [NOT] DISTINCT FROM` cell is a concrete boolean
-- (never NULL) — the whole point of these operators vs `=`/`<>`.
-- @setup
CREATE TABLE idf (id int4, a int4, b int4);
INSERT INTO idf (id, a, b) VALUES
  (1, NULL, NULL),  -- both NULL    → not distinct (eq), distinct false
  (2, NULL, 5),     -- left NULL     → distinct
  (3, 5, NULL),     -- right NULL    → distinct
  (4, 5, 5),         -- equal         → not distinct
  (5, 5, 7);         -- unequal       → distinct
-- @test
SELECT
  id,
  a IS DISTINCT FROM b AS is_distinct,
  a IS NOT DISTINCT FROM b AS is_not_distinct
FROM idf
ORDER BY id;
