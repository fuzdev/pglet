-- An `ARRAY[…]` branch whose elements the branch reader types only in part — one over a
-- scalar subquery whose own column is a guess — is not typed
-- as the `text[]` an all-unknown list resolves to: beside a typed array branch it keeps
-- the set at the written reading, which answers 18.6's values. Cast to `text` so the
-- declared types compared are both `text` (the array's own is `EDGES.md` row 54's band).
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8, tx TEXT);
INSERT INTO t VALUES (1, 1, 1.5, 'a'), (2, NULL, 2.5, 'b'), (3, 5, NULL, NULL);
-- @test
SELECT COALESCE(ARRAY[i + 0], ARRAY[f + (SELECT * FROM unnest(ARRAY[1]))])::text AS c1,
  COALESCE(ARRAY[length(tx)], ARRAY[f + (SELECT * FROM unnest(ARRAY[1]))])::text AS c2,
  (CASE WHEN k = 1 THEN ARRAY[i + 0] ELSE ARRAY[f + (SELECT * FROM unnest(ARRAY[1]))] END)::text AS c3
FROM t ORDER BY k;
