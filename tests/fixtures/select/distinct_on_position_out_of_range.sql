-- A positional `DISTINCT ON (n)` whose ordinal is past the select-list length fails
-- loud (42P10, Postgres: "DISTINCT ON position 5 is not in select list") rather than
-- the old 42601 syntax error. No-FROM select — the whole ON key is the sort key.
-- @expect-error: DISTINCT ON position out of range (42P10)
-- @test
SELECT DISTINCT ON (5) 1;
