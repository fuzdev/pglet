-- @setup
CREATE TABLE a (num INT, name TEXT);
CREATE TABLE b (num INT, val TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');

-- @test
-- @expect-error: no output column named `num` (aliased x/y), so ORDER BY num falls to the ambiguous FROM scope (42702)
SELECT a.num AS x, b.num AS y FROM a JOIN b ON a.num = b.num ORDER BY num;
