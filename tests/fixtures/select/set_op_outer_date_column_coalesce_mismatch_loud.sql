-- The set operation is `date` through the enclosing column, and `COALESCE` has no common
-- type for `date` and `integer`.
-- @setup
CREATE TABLE socm (d DATE);
INSERT INTO socm VALUES ('2020-01-02');
-- @expect-error: COALESCE types date and integer cannot be matched
-- @test
SELECT coalesce((SELECT d EXCEPT SELECT '2021-01-01'), 2) FROM socm;
