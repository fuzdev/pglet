-- `ORDER BY` over a function of an aggregate the select list does not carry — a junk
-- column of the grouping node, evaluated with the select list.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT a FROM u GROUP BY a ORDER BY abs(sum(b) - 5) DESC, a;
