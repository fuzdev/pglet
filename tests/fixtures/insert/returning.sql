-- @setup
CREATE TABLE users (id INT, name TEXT, age INT);

-- @test
INSERT INTO users VALUES (1, 'Alice', 30) RETURNING id, name;
