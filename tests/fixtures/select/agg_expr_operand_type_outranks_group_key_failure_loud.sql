-- An aggregate's operand type is refused before the groups are formed, as 18.6 refuses
-- it in parse analysis: the `GROUP BY` key would divide by the `b = 0` row (`22012`), and
-- the statement is `42883`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, t TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'y');
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT sum(t) FROM u GROUP BY 1/b;
