-- The refusal polarity of the same rule, and the one that says the fix is a *binding*
-- change rather than a sort-placement one. `z` is an output alias and no relation
-- carries it, so SQL92 never reaches the key and SQL99 resolution finds nothing:
-- `42703` on 18.6. pglet answered rows, having exempted every bare name in an
-- `ORDER BY` from the plan-time binding gate rather than only a key that *is* one.
--
-- The accept polarity is the whole-key spelling `ORDER BY z`, which both engines
-- answer. Its fixture is `select/distinct_on_alias_key.sql`, whose `ORDER BY d` over
-- `dept AS d` names an alias no relation carries — the same namespace this refuses in,
-- reached by a key of the shape SQL92 does enter. (Not
-- `select/order_by_output_alias_precedence.sql`: its `num` is a base column of both
-- FROM relations, so it exhibits alias *precedence* over a real column rather than an
-- output name nothing else carries.)
-- @setup
CREATE TABLE onz (q INT);
INSERT INTO onz VALUES (3), (1), (2);

-- @expect-error: column z does not exist (42703)
-- @test
SELECT q*10 AS z FROM onz ORDER BY z+0;
