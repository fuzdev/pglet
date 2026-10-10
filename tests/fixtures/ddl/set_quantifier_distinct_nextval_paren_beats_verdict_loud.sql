-- Where the verdict sits against the call's **closing paren**, at the third route — the
-- contract `functions/set_quantifier_distinct_from_unnest_paren_beats_verdict_loud.sql`
-- pins at the FROM route and `parse_function_call` has structurally. 18.6 scans the whole
-- call before parse analysis judges it, so a syntax error standing where the `)` should
-- be wins; raised above the `expect(RParen)` the classifier answers `42809` to this
-- statement instead — the one mutation that reddens this fixture. It is **green on the
-- build that read no quantifier here at all**, where the `42601` came from `DISTINCT`
-- meeting the expression parser: it guards the verdict's position, not its existence,
-- and the sibling `_column_default_loud` guards the existence. `42601` on both.
-- @setup
CREATE SEQUENCE sq_npv_seq;
-- @test
-- @expect-error: syntax error at or near "junk"
CREATE TABLE sq_npv (a BIGINT DEFAULT nextval(DISTINCT 'sq_npv_seq' junk));
