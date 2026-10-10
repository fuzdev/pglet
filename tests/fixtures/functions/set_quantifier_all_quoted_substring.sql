-- The control that identifies `planner::grammar_only_name`'s rule as **`pg_proc`
-- membership** rather than grammar membership, and the one a wrong widening breaks
-- first. `substring` is a `gram.y` production on 18.6 *and* a `pg_proc` entry, so its
-- quoted spelling leaves the keyword route and resolves as an ordinary function there —
-- which is why `substring` must stay **out** of that set even though `trim`, its
-- neighbour on the grammar list, is in it (`pg_proc` holds `btrim`, not `trim`). Adding
-- it would answer 42601 here against this row. Its `DISTINCT` twin
-- (`set_quantifier_distinct_quoted_substring_loud.sql`) is the other polarity.
-- @test
SELECT "substring"(ALL 'abcdef', 2) AS s;
