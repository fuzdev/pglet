-- A quoted lower-case `"now"` names the real `pg_proc` entry on 18.6, so the argument
-- list is judged the same way there; pglet's fold leaves the quoted spelling byte-equal
-- to the folded unquoted one, so it reaches the same arm. 42883 on both.
-- @test
-- @expect-error: function now(integer) does not exist
SELECT "now"(1);
