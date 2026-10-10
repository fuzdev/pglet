-- @setup
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol');
DELETE FROM users WHERE id > 1;

-- @test
SELECT * FROM users ORDER BY id;
