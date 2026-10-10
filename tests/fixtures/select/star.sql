-- @setup
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');

-- @test
SELECT * FROM users ORDER BY id;
