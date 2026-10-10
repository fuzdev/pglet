-- @setup
CREATE TABLE multi_type (id INT, name TEXT, price FLOAT, active BOOL, count BIGINT);
INSERT INTO multi_type VALUES (1, 'test', 19.99, true, 1000000000);

-- @test
SELECT * FROM multi_type;
