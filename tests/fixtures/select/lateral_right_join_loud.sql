-- A `LATERAL` item on the right of a `RIGHT` or `FULL` join is refused `0A000`. 18.7
-- answers this one, whose item names nothing to its left, and refuses one that does
-- (`42P10`); pglet does not tell the two apart.
-- @setup
CREATE TABLE lp (id int, name text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b');
-- @expect-error: a LATERAL item on the right of a RIGHT or FULL JOIN not supported
-- @test
SELECT * FROM lp p RIGHT JOIN LATERAL (SELECT 1 AS a) x ON true ORDER BY 1;
