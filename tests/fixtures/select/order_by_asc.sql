-- @setup
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (3, 'Carol'), (1, 'Alice'), (2, 'Bob');

-- @test
SELECT * FROM users ORDER BY name;
