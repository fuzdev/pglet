-- @setup
CREATE TABLE a (num INT, name TEXT);
CREATE TABLE b (num INT, val TEXT);
INSERT INTO a VALUES (1, 'a1'), (2, 'a2'), (3, 'a3');
INSERT INTO b VALUES (1, 'b1'), (2, 'b2');

-- @test
SELECT num FROM a JOIN b USING (num) ORDER BY num;
