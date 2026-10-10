-- The inverse of `unnest_alias_names_the_column.sql`: once the alias names the column,
-- the function name no longer does. `42703` on both engines, so the SQLSTATE is
-- compared.
--
-- Kept as its own file because it is the half a one-directional fix leaves behind — a
-- rule that *added* the alias as a second name for the column, rather than replacing
-- the default, turns its sibling green and this one red.
-- @setup

-- @test
-- @expect-error: 42703 — with an alias, the single unnest column is named after it, not `unnest`
SELECT unnest FROM unnest(ARRAY['a', 'b']) AS v;
