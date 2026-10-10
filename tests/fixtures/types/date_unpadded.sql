-- Postgres accepts unpadded date fields (2026-1-1 = 2026-01-01, 2026-6-15 = 2026-06-15);
-- pglet parses them the same and renders the padded output form.
-- @test
SELECT '2026-1-1'::date AS a, '2026-6-15'::date AS b, '2026-01-01'::date AS c;
