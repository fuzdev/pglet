-- @setup
CREATE TABLE a (num INT, name TEXT);
CREATE TABLE b (num INT, val TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');

-- @test
-- @expect-error: bare `num` in WHERE matches both a and b (42702)
SELECT a.name FROM a JOIN b ON a.num = b.num WHERE num > 0;
