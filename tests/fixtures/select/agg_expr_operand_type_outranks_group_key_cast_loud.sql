-- An aggregate's operand type is refused before the groups are formed: the `GROUP BY`
-- key's cast is itself refused (`42846`) once a row is read, and 18.6, which analyses the
-- select list first, raises the `sum`'s `42883`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, t TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'y');
-- @expect-error: no aggregate of that name takes the operand's type
-- @test
SELECT sum(t) FROM u GROUP BY a::jsonb;
