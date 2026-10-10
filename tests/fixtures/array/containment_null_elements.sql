-- Native-array `@>` / `<@` with NULL elements, both directions.
--
-- Postgres's `array_contain_compare` (`arrayfuncs.c`) iterates the **contained**
-- array and, on a NULL there, breaks with `result = false` — "we assume the
-- comparison operator is strict, so a NULL can't match anything". A NULL in the
-- **container** is instead skipped (`continue`), so it neither matches nor poisons.
-- The two asymmetric rules are what this locks.
--
-- pglet needs no special case for either: `values_equal` returns `None` for any NULL
-- operand, which fails the inner `any` (container NULL) and therefore the outer `all`
-- (contained NULL). That equivalence was asserted in the docs as a known
-- *divergence* for a while without anyone checking it against the source — it was
-- conformant the whole time. Locked here so the claim stops being a matter of
-- reading.
-- @test
SELECT
  ('{1,2}'::int[] @> '{NULL}'::int[]) AS null_in_contained,
  ('{1,NULL}'::int[] @> '{1}'::int[]) AS null_in_container,
  ('{1,NULL}'::int[] @> '{NULL}'::int[]) AS null_matches_no_null,
  ('{NULL}'::int[] @> '{NULL}'::int[]) AS both_null,
  ('{1,2}'::int[] @> '{}'::int[]) AS empty_contained,
  ('{NULL}'::int[] <@ '{1,NULL}'::int[]) AS null_contained_by,
  ('{}'::int[] <@ '{NULL}'::int[]) AS empty_contained_by
;
