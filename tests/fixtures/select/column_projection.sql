-- @setup
CREATE TABLE users (id INT, name TEXT, age INT);
INSERT INTO users VALUES (1, 'Alice', 30);

-- @test
SELECT name, id FROM users;
