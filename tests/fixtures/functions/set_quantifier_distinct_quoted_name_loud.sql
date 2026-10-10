-- A quoted lower-case name is the same function on both engines — `pg_proc` holds
-- `upper` — so the quantifier verdict is reached identically through the quoted
-- spelling. 42809 on both; pglet answered `A`. (The quoted **upper**-case
-- `"UPPER"(DISTINCT 'a')` is a different, pre-existing class: 42883 there against
-- pglet's 0A000 for a name it models no function for.)
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
SELECT "upper"(DISTINCT 'a');
