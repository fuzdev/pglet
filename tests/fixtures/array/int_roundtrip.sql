-- @setup
CREATE TABLE nums (id INT, vals INT[]);
INSERT INTO nums VALUES (1, ARRAY[10, 20, 30]), (2, '{}'), (3, NULL);

-- @test
SELECT id, vals FROM nums ORDER BY id;
