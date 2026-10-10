-- `IS` binds **tighter** than `NOT`/`AND` and **looser** than a comparison
-- (`gram.y`'s `%nonassoc IS`), so `a = b IS TRUE` is `(a = b) IS TRUE` and
-- `NOT x IS TRUE` is `NOT (x IS TRUE)`. Each column is a spelling that answers the
-- other way under the opposite grouping.
-- @test
SELECT
  1 = 1 IS TRUE AS cmp_binds_tighter,
  1 = 2 IS TRUE AS cmp_binds_tighter_false,
  NOT true IS TRUE AS not_binds_looser,
  NOT false IS TRUE AS not_binds_looser_true,
  true AND false IS TRUE AS and_binds_looser,
  'a' LIKE 'a' IS TRUE AS like_binds_tighter,
  NULL IS TRUE IS TRUE AS chained;
