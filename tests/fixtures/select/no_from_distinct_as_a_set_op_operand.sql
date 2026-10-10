-- The quantifier is applied in `parse_select_leaf`, so **both** routes out of a no-FROM
-- leaf carry it — `parse_select_inner`'s own arm for a single select, and `finalize_leaf`
-- for a set-op operand, which is this. Applying it at the arm instead would leave this
-- spelling deduping nothing: four rows here against 18.6's three.
--
-- `UNION ALL` is load-bearing — a plain `UNION` dedups the whole chain and would answer
-- three rows either way, hiding the operand's own quantifier.
-- @test
SELECT DISTINCT unnest(ARRAY[2, 1, 2]) AS q UNION ALL SELECT 9 ORDER BY 1;
