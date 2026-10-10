-- @setup
CREATE TABLE a (num INT, name TEXT);
CREATE TABLE b (num INT, val TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');

-- @test
-- @expect-error: bare `num` matches both a and b in the join projection (42702)
SELECT num FROM a JOIN b ON a.num = b.num;
