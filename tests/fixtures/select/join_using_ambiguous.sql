-- @setup
CREATE TABLE a1 (num INT, x TEXT);
CREATE TABLE a2 (num INT, y TEXT);
CREATE TABLE a3 (num INT, z TEXT);
INSERT INTO a1 VALUES (1, 'x1');
INSERT INTO a2 VALUES (1, 'y1');
INSERT INTO a3 VALUES (1, 'z1');

-- @test
-- @expect-error: `num` is ambiguous in the left side (a1 CROSS JOIN a2) of USING
SELECT * FROM a1 CROSS JOIN a2 JOIN a3 USING (num);
