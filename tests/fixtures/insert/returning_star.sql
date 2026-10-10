-- @setup
CREATE TABLE users (id INT, name TEXT, age INT);

-- @test
INSERT INTO users VALUES (1, 'Alice', 30), (2, 'Bob', 25) RETURNING *;
