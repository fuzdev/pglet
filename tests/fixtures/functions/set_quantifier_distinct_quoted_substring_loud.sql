-- The bonus the set-quantifier close took by exclusion: pglet answered `bcdef` here
-- where 18.6 refuses, because the quoted spelling leaves `parse_substring`'s keyword
-- route, reaches the ordinary dispatch, and — `substring` being a real `pg_proc` entry
-- on 18.6 — is judged there by the same `agg_distinct` arm. 42809 on both.
-- Together with `set_quantifier_all_quoted_substring.sql` this is what a widening of
-- `planner::grammar_only_name` to `substring` reddens: it would answer 42601.
-- @test
-- @expect-error: DISTINCT specified, but substring is not an aggregate function
SELECT "substring"(DISTINCT 'abcdef', 2);
