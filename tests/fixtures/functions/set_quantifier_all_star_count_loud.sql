-- The `ALL` polarity of the wildcard bound, over the one name whose `*` spelling is
-- legal without a quantifier — `count(*)` answers on both engines, so this fixture
-- separates "the `*` is refused" from "the pairing is refused". 42601 on both.
-- @setup
CREATE TABLE sq_acs (a INT);
INSERT INTO sq_acs VALUES (1), (2);
-- @test
-- @expect-error: syntax error at or near "*"
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the `*`, pglet at the closing paren
SELECT count(ALL *) FROM sq_acs;
