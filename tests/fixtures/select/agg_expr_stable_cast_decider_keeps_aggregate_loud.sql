-- A `date` to `timestamptz` cast is stable in 18.6, so a condition over it is no
-- constant.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN '2020-01-01'::date::timestamptz IS NOT NULL THEN 0 ELSE sum(1/b) END FROM u;
