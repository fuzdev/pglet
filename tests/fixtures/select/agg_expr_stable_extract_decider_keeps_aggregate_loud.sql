-- `extract` from a `timestamptz` is stable in 18.6 (from a `date` or a `timestamp` it is
-- not), so a condition over it is no constant.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN extract(year FROM '2020-01-01 00:00+00'::timestamptz) = 2020 THEN 0 ELSE sum(1/b) END FROM u;
