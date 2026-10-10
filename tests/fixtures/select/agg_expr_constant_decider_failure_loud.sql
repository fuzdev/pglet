-- A failing constant the scalar walk reaches is raised: the condition is evaluated for
-- the one group, and the `sum` here cannot fail.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN 1/0 > 0 THEN 1 ELSE sum(a) END FROM u;
