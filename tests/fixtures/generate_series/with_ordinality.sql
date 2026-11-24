-- `generate_series` takes `WITH ORDINALITY` as `unnest` does: the series column keeps the
-- alias's name, and the ordinality column is `ordinality` or the alias list's second name.
-- @test
SELECT g, g.ordinality, s.v, s.n FROM generate_series(4, 8, 2) WITH ORDINALITY AS g
  JOIN generate_series(10, 6, -2) WITH ORDINALITY AS s(v, n) ON s.n = g.ordinality
ORDER BY g.ordinality;
