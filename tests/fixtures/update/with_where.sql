-- @setup
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');
UPDATE users SET name = 'Alicia' WHERE id = 1;

-- @test
SELECT * FROM users ORDER BY id;
