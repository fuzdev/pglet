-- @setup
CREATE TABLE a (num INT, name TEXT);
CREATE TABLE b (num INT, val TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');

-- @test
SELECT name FROM a JOIN b ON a.num = b.num ORDER BY name;
