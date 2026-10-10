-- Canonical key order is observable through `<` / `ORDER BY`, not only through
-- rendering — and the two orderings involved are deliberately *different*.
--
-- Postgres stores an object's keys by `lengthCompareJsonbString` (length first, then
-- bytewise) but compares two objects pairwise in that stored order using `varstr_cmp`
-- (lexical). So the storage rule leaks into the comparison result: the values below
-- are chosen so that a *lexicographic* storage order would flip the answer.
--
--   {"z":1,"aa":2}  stores [z, aa]   {"y":3,"zz":4}  stores [y, zz]
--   first keys compared: "z" vs "y"  →  greater  →  `<` is false, and row 2 sorts first
--
-- Under a lexicographic storage order they would store [aa, z] and [y, zz], the first
-- comparison would be "aa" vs "y", and both answers would flip. The sibling
-- `key_order_*` fixtures all use equal-length keys, where the two rules agree — this
-- is the case that separates them.
-- @setup
CREATE TABLE jkoc (id INT, d JSONB);
INSERT INTO jkoc VALUES
  (1, '{"z":1,"aa":2}'),
  (2, '{"y":3,"zz":4}'),
  (3, '{"aa":2,"z":1}');
-- @test
SELECT
  id,
  d,
  ('{"z":1,"aa":2}'::jsonb < '{"y":3,"zz":4}'::jsonb) AS length_first_order,
  ('{"z":1,"aa":2}'::jsonb = '{"aa":2,"z":1}'::jsonb) AS spelling_irrelevant
FROM jkoc ORDER BY d, id;
