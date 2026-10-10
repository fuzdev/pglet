-- Same category is NOT the same as convertible. `date` and `time` are both
-- `typcategory` D, so the category check passes and the failure lands one step later,
-- when Postgres coerces each branch to the resolved type: `42846`
-- ("COALESCE could not convert type time without time zone to date"), not `42804`.
-- The two codes are different facts and the engine must keep them apart.
-- @setup
CREATE TABLE cu (d DATE, t TIME);
INSERT INTO cu VALUES ('2026-01-01', '12:00:00');
-- @test
-- @expect-error: same category, no conversion (42846)
SELECT COALESCE(d, t) FROM cu;
